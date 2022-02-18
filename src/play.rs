use std::collections::HashMap;
use std::time::Duration;

use topaz_tak::{
    eval::Weights6,
    search::{search, SearchInfo},
    Position,
};

use super::*;
use discord::model::UserId;

pub const TOPAZ_ID: UserId = UserId(211376698778714123);
pub const TAK_BOT_ID: UserId = UserId(793658103668539424);

static LINK_START: &'static str = "<https://ptn.ninja/";

#[derive(Default)]
pub struct Matches {
    pub matches: HashMap<ChannelId, AsyncGameState>,
}

impl Matches {
    pub fn update_rooms(&mut self, discord: &Discord) -> Result<()> {
        let channels = discord
            .get_server_channels(TAK_TALK)
            .expect("Failed to get Tak server channels!");
        for mut game in channels
            .iter()
            .filter_map(|chan| AsyncGameState::try_new(chan))
        {
            // If we are not yet tracking this game room
            if !self.matches.contains_key(&game.channel_id) {
                let messages = discord.get_messages(
                    game.channel_id,
                    discord::GetMessages::MostRecent,
                    Some(16),
                )?;
                // If we didn't get the full information that we needed request a link
                if game.search_room(&messages).is_none() {
                    game.request_link(discord)?;
                } else {
                    game.make_move(discord).expect("Unable to make move!");
                }
                self.matches.insert(game.channel_id, game);
            }
        }
        Ok(())
    }
}

pub struct AsyncGameState {
    pub channel_id: ChannelId,
    pub board: Option<Board6>,
    player1: String,
    player2: String,
}

impl AsyncGameState {
    pub fn try_new(channel: &PublicChannel) -> Option<Self> {
        let mut iter = channel.name.split("-🆚-");
        let p1 = iter.next()?;
        let p2 = iter.next()?;
        if iter.next().is_some() {
            None
        } else {
            if p1 == TOPAZ || p2 == TOPAZ {
                Some(Self {
                    channel_id: channel.id,
                    board: None,
                    player1: p1.to_string(),
                    player2: p2.to_string(),
                })
            } else {
                None
            }
        }
    }
    pub fn topaz_turn(&self) -> Option<bool> {
        let board = self.board.as_ref()?;
        let color = board.side_to_move();
        let b = match color {
            Color::White => TOPAZ == &self.player1,
            Color::Black => TOPAZ == &self.player2,
        };
        Some(b)
    }
    pub fn invalidate_board(&mut self) {
        self.board = None;
    }
    pub fn do_message(&mut self, message: &Message, discord: &Discord) {
        if message.content.starts_with("Your turn ") {
            if message
                .mentions
                .iter()
                .find(|x| x.id == play::TOPAZ_ID)
                .is_some()
            {
                if let Some(true) = self.topaz_turn() {
                    let new_board =
                        play_async_move(self.board.take().unwrap(), message.channel_id, &discord)
                            .expect("Failed to send message");
                    self.board = Some(new_board);
                } else {
                    self.request_link(discord).unwrap();
                }
            }
        } else if message.content.starts_with("!tak undo") {
            self.invalidate_board();
        } else if message.content.starts_with("!tak rematch") {
            self.invalidate_board();
        } else if message.content.starts_with("Invalid move.") {
            self.invalidate_board();
        } else if message.content.starts_with("You are not ") {
            self.invalidate_board();
        } else if message.author.id == TAK_BOT_ID && message.content.starts_with(LINK_START) {
            let link: String = message
                .content
                .chars()
                .filter(|&c| c != '<' && c != '>')
                .collect();
            self.board = handle_link(&link);
            self.make_move(discord).expect("Unable to make move!");
        } else if message.content.starts_with("!topaz position") {
            let s = format!("This is the position, right? \n{:?}", self.board);
            discord
                .send_message(message.channel_id, &s, "", false)
                .expect("Failed to send message!");
        } else {
            if let Some(ref mut board) = self.board {
                if let Some(m) =
                    super::parse_move(&message.content, Board6::SIZE, board.side_to_move())
                {
                    board.do_move(m);
                }
            }
        }
    }
    pub fn make_move(&mut self, discord: &Discord) -> Option<()> {
        if self.topaz_turn()? {
            if self.board.as_ref()?.game_result().is_some() {
                return Some(());
            }
            let new_board = play_async_move(self.board.take().unwrap(), self.channel_id, &discord)
                .expect("Failed to send message");
            self.board = Some(new_board);
        }
        Some(())
    }
    pub fn request_link(&self, discord: &Discord) -> Result<()> {
        discord.send_message(self.channel_id, "!tak link", "", false)?;
        std::thread::sleep(Duration::from_secs(3));
        Ok(())
    }
    pub fn search_room(&mut self, messages: &[Message]) -> Option<()> {
        let game_link = find_link(messages)?;
        let mut board = handle_link(&game_link)?;
        let mut extra_moves = Vec::new();
        for message in messages.iter() {
            if message.content.starts_with(LINK_START) {
                break;
            }
            if super::parse_move(&message.content, Board6::SIZE, board.side_to_move()).is_some() {
                extra_moves.push(&message.content);
            }
        }
        for mv in extra_moves.into_iter().rev() {
            let m = super::parse_move(&mv, Board6::SIZE, board.side_to_move()).unwrap();
            board.do_move(m);
        }
        self.board = Some(board);
        Some(())
    }
}

fn handle_link(game_link: &str) -> Option<Board6> {
    let ptn = super::get_ptn_string(&game_link).expect("Valid ptn string!");
    let game = super::parse_game(&ptn)?;
    match game {
        (TakGame::Standard6(mut board), moves) => {
            for m in moves {
                board.do_move(m);
            }
            return Some(board);
        }
        _ => {
            return None;
        }
    }
}

fn find_link(messages: &[Message]) -> Option<String> {
    for message in messages.iter() {
        println!("{:?}: {}", message.author, message.content);
        if message.content.starts_with("!tak undo") {
            break;
        } else if message.content.starts_with("!tak rematch") {
            break;
        } else if message.content.starts_with("Invalid move.") {
            break;
        } else if message.content.starts_with("You are not ") {
            break;
        } else if message.author.id == TAK_BOT_ID && message.content.starts_with(LINK_START) {
            let link = message
                .content
                .chars()
                .filter(|&c| c != '<' && c != '>')
                .collect();
            return Some(link);
        }
    }
    None
}

pub fn play_async_move(mut board: Board6, channel: ChannelId, discord: &Discord) -> Result<Board6> {
    let mut tinue_search = TinueSearch::new(board).limit(NODE_LIMIT * 5).quiet();
    let best_move = if Some(true) == tinue_search.is_tinue() {
        let pv_move = tinue_search.principal_variation().into_iter().next();
        board = tinue_search.board;
        if let Some(mv) = pv_move {
            format!("{}\"", mv.to_ptn::<Board6>())
        } else {
            // Maybe it's just one ply?
            let road_move = find_road_move(&mut board);
            if let Some(mv) = road_move {
                mv.to_ptn::<Board6>()
            } else {
                return Err(anyhow!("Failed getting tinue pv search / road move!"));
            }
        }
    } else {
        board = tinue_search.board;
        let mut info = SearchInfo::new(6, 1000000).max_time(30);
        let mut eval = Weights6::default();
        if board.move_num() <= 6 {
            eval.add_noise();
        }
        let best_move = search(&mut board, &eval, &mut info)
            .and_then(|x| x.best_move())
            .ok_or_else(|| anyhow!("No best move from game search!"))?;
        let mv = GameMove::try_from_ptn(&best_move, &board).unwrap();
        let rev = board.do_move(mv);
        board.null_move();
        let mut moves = Vec::new();
        let tak = board.can_make_road(&mut moves, None).is_some();
        board.rev_null_move();
        board.reverse_move(rev);
        if tak {
            format!("{}'", best_move)
        } else {
            best_move
        }
    };
    std::thread::sleep(Duration::from_secs(5));
    discord.send_message(channel, &best_move, "", false)?;
    Ok(board)
}
