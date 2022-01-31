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

pub struct AsyncGameState {
    pub topaz_turn: Option<bool>,
    pub board: Option<Board6>,
}

impl AsyncGameState {
    fn new(topaz_turn: Option<bool>, board: Option<Board6>) -> Self {
        Self { topaz_turn, board }
    }
}

pub fn search_room(
    channel: ChannelId,
    discord: &Discord,
    only_topaz: bool,
    use_messages: Option<Vec<Message>>,
) -> Result<AsyncGameState> {
    let messages = if let Some(m) = use_messages {
        m
    } else {
        discord.get_messages(channel, discord::GetMessages::MostRecent, Some(10))?
    };
    // Search most recent first
    let topaz_turn = my_turn(&messages);
    if only_topaz && topaz_turn.is_none() {
        return Err(anyhow!("Could not determine which player's turn it is"));
    }
    let game_link = find_link(&messages);
    let board = if let Some(link) = game_link {
        let ptn = super::get_ptn_string(&link)?;
        let game = super::parse_game(&ptn).ok_or_else(|| anyhow!("Could not read ptn!"))?;
        match game {
            (TakGame::Standard6(mut board), moves) => {
                for m in moves {
                    board.do_move(m);
                }
                let mut extra_moves = Vec::new();
                for message in messages.iter() {
                    if message.content.starts_with(LINK_START) {
                        break;
                    }
                    if super::parse_move(&message.content, Board6::SIZE, board.side_to_move())
                        .is_some()
                    {
                        extra_moves.push(&message.content);
                    }
                }
                for mv in extra_moves.into_iter().rev() {
                    let m = super::parse_move(&mv, Board6::SIZE, board.side_to_move()).unwrap();
                    board.do_move(m);
                }
                board
            }
            _ => {
                return Err(anyhow!("Unsupported game size!"));
            }
        }
    } else {
        discord.send_message(channel, "!tak link", "", false)?;
        std::thread::sleep(Duration::from_secs(5));
        let messages = discord.get_messages(channel, discord::GetMessages::MostRecent, Some(5))?;
        let link =
            find_link(&messages).ok_or_else(|| anyhow!("Takbot did not respond with link!"))?;
        let ptn = super::get_ptn_string(&link)?;
        let game = super::parse_game(&ptn).ok_or_else(|| anyhow!("Could not read ptn!"))?;
        match game {
            (TakGame::Standard6(mut board), moves) => {
                for m in moves {
                    board.do_move(m);
                }
                board
            }
            _ => {
                return Err(anyhow!("Unsupported game size!"));
            }
        }
    };
    Ok(AsyncGameState::new(topaz_turn, Some(board)))
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

pub fn my_turn(messages: &[Message]) -> Option<bool> {
    // let messages = discord.get_messages(channel, discord::GetMessages::MostRecent, Some(10))?;
    for message in messages.iter() {
        if message.author.id == TAK_BOT_ID && message.content.starts_with("Your turn ") {
            if let Some(user) = message.mentions.iter().next() {
                if user.id == TOPAZ_ID {
                    return Some(true);
                } else {
                    return Some(false);
                }
            } else {
                return None;
            }
        }
    }
    None
}

pub fn play_async_move(mut board: Board6, channel: ChannelId, discord: &Discord) -> Result<()> {
    let mut tinue_search = TinueSearch::new(board).limit(NODE_LIMIT * 5).quiet();
    let best_move = if Some(true) == tinue_search.is_tinue() {
        let pv_move = tinue_search.principal_variation().into_iter().next();
        if let Some(mv) = pv_move {
            mv.to_ptn::<Board6>()
        } else {
            // Maybe it's just one ply?
            let road_move = find_road_move(&mut tinue_search.board);
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
        // let message = { 0 };
        search(&mut board, &eval, &mut info)
            .and_then(|x| x.best_move())
            .ok_or_else(|| anyhow!("No best move from game search!"))?
    };
    std::thread::sleep(Duration::from_secs(5));
    discord.send_message(channel, &best_move, "", false)?;
    Ok(())
}
