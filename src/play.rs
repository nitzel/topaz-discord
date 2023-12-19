use std::collections::HashMap;

use topaz_tak::{
    eval::{Evaluator, Weights5, Weights6},
    search::{search, SearchInfo, SearchOutcome},
    transposition_table::HashTable,
    Position, TakGame, TimeBank,
};

use super::*;
use serenity::model::id::ChannelId;

const MAX_DEPTH: usize = 20;
const GOAL_TIME: u64 = 20_000;

static GAME_TABLE: once_cell::sync::Lazy<HashTable> =
    once_cell::sync::Lazy::new(|| HashTable::new(2 << 19));

#[derive(Default)]
pub struct Matches {
    pub matches: HashMap<ChannelId, AsyncGameState>,
}

impl Matches {
    // pub fn update_rooms(&mut self, discord: &Discord) -> Result<()> {
    //     std::thread::sleep(Duration::from_secs(5));
    //     let channels = discord
    //         .get_server_channels(TAK_TALK)
    //         .expect("Failed to get Tak server channels!");
    //     for game in channels
    //         .iter()
    //         .filter_map(|chan| AsyncGameState::try_new(chan))
    //     {
    //         self.track_room(discord, game)?;
    //     }
    //     dbg!(self.matches.len());
    //     Ok(())
    // }
    // pub fn track_room(&mut self, discord: &Discord, mut game: AsyncGameState) -> Result<()> {
    //     // If we are not yet tracking this game room
    //     if !self.matches.contains_key(&game.channel_id) {
    //         let messages = discord.get_messages(
    //             game.channel_id,
    //             discord::GetMessages::MostRecent,
    //             Some(16),
    //         )?;
    //         // If we didn't get the full information that we needed request a link
    //         if game.search_room(&messages).is_none() {
    //             game.request_link(discord)?;
    //         } else {
    //             game.make_move(discord).expect("Unable to make move!");
    //         }
    //         self.matches.insert(game.channel_id, game);
    //     }
    //     Ok(())
    // }
    // pub fn untrack_room(&mut self, discord: &Discord, chan: &PublicChannel) -> Result<()> {
    //     let _ = self.matches.remove(&chan.id);
    //     Ok(())
    // }
}

#[derive(PartialEq, Debug, Clone, Copy)]
enum MoveStatus {
    Topaz,
    Unknown,
    Opponent,
}

impl Default for MoveStatus {
    fn default() -> Self {
        MoveStatus::Unknown
    }
}

#[derive(Default)]
pub struct AsyncGameState {
    pub board: Option<TakGame>,
    move_status: MoveStatus,
    undo_request: bool,
    dirty: bool,
}

impl AsyncGameState {
    pub fn get_copy(&self) -> Self {
        // Hack because I didn't implement clone in the main library
        let board = match &self.board {
            Some(TakGame::Standard5(b)) => Some(TakGame::Standard5(b.clone())),
            Some(TakGame::Standard6(b)) => Some(TakGame::Standard6(b.clone())),
            Some(TakGame::Standard7(b)) => Some(TakGame::Standard7(b.clone())),
            _ => None,
        };
        Self {
            board,
            move_status: self.move_status,
            undo_request: self.undo_request,
            dirty: false,
        }
    }
    pub fn is_dirty(&self) -> bool {
        self.dirty
    }
    pub fn set_dirty(&mut self) {
        self.dirty = true;
    }
    pub fn try_apply_move(&mut self, ptn_move: &str) -> Result<()> {
        if let Some(b) = self.board.as_mut() {
            let res = match b {
                TakGame::Standard5(board) => board.make_ptn_moves(&[ptn_move]),
                TakGame::Standard6(board) => board.make_ptn_moves(&[ptn_move]),
                TakGame::Standard7(board) => board.make_ptn_moves(&[ptn_move]),
                _ => todo!(),
            };
            return res.ok_or_else(|| anyhow::anyhow!("Failed to apply ptn move"));
        }
        Err(anyhow::anyhow!("Unknown board state"))
    }
    pub fn set_board(&mut self, game: TakGame) {
        self.board = Some(game);
    }
    pub fn needs_action(&mut self) {
        self.move_status = MoveStatus::Topaz;
    }
    pub fn waiting(&mut self) {
        self.move_status = MoveStatus::Opponent;
    }
    // pub fn topaz_turn(&self) -> Option<bool> {
    //     let board = self.board.as_ref()?;
    //     let color = board.side_to_move();
    //     let b = match color {
    //         Color::White => TOPAZ == &self.player1,
    //         Color::Black => TOPAZ == &self.player2,
    //     };
    //     Some(b)
    // }
    // pub fn topaz_turn(&self) -> Option<bool> {
    //     if self.board.is_none() {
    //         return None;
    //     }
    //     Some(self.owes_move)
    // }
    pub fn invalidate_board(&mut self) {
        self.board = None;
    }
    pub fn is_unknown_state(&self) -> bool {
        self.move_status == MoveStatus::Unknown
    }
    pub fn is_topaz_move(&self) -> bool {
        self.move_status == MoveStatus::Topaz
    }
    pub async fn do_message(&mut self, context: &Context, message: Message) {
        if self.move_status == MoveStatus::Topaz && self.board.is_some() {
            let new_board = match self.board.take().unwrap() {
                TakGame::Standard5(board) => TakGame::Standard5(
                    play_async_move::<Weights5>(board, context, message.channel_id)
                        .await
                        .expect("Failed to send message"),
                ),
                TakGame::Standard6(board) => TakGame::Standard6(
                    play_async_move::<Weights6>(board, context, message.channel_id)
                        .await
                        .expect("Failed to send message"),
                ),
                _ => todo!(),
            };
            self.board = Some(new_board);
        }
        // if message.content.starts_with("Your turn ") {
        // if message
        //     .mentions
        //     .iter()
        //     .find(|x| x.id == play::TOPAZ_ID)
        //     .is_some()
        // {
        //     if let Some(true) = self.topaz_turn() {
        //         let new_board = match self.board.take().unwrap() {
        //             TakGame::Standard5(board) => TakGame::Standard5(
        //                 play_async_move::<Weights5>(board, message.channel_id, &discord)
        //                     .expect("Failed to send message"),
        //             ),
        //             TakGame::Standard6(board) => TakGame::Standard6(
        //                 play_async_move::<Weights6>(board, message.channel_id, &discord)
        //                     .expect("Failed to send message"),
        //             ),
        //             _ => todo!(),
        //         };
        //         self.board = Some(new_board);
        //     } else {
        //         self.request_link(discord).unwrap();
        //     }
        // }
        // } else if message.content.starts_with("!tak undo") {
        //     if self.undo_request {
        //         if message.author.id != TOPAZ_ID {
        //             self.undo_request = false;
        //         }
        //     }
        //     self.invalidate_board();
        // } else if message.content.starts_with("!tak rematch") {
        //     self.invalidate_board();
        // } else if message.content.starts_with("Invalid move.") {
        //     self.invalidate_board();
        // } else if message.content.starts_with("You are not ") {
        //     self.invalidate_board();
        // } else if message.content.starts_with("!topaz undo") {
        //     self.undo_request = true;
        //     std::thread::sleep(Duration::from_secs(2));
        //     discord
        //         .send_message(message.channel_id, "!tak undo", "", false)
        //         .unwrap();
        // } else if message.author.id == TAK_BOT_ID && message.content.starts_with(LINK_START) {
        //     let link: String = message
        //         .content
        //         .chars()
        //         .filter(|&c| c != '<' && c != '>')
        //         .collect();
        //     self.board = handle_link(&link);
        //     self.make_move(discord).expect("Unable to make move!");
        // } else if message.content.starts_with("!topaz position") {
        //     let s = format!("This is the position, right? \n{}", debug_tps(&self.board));
        //     discord
        //         .send_message(message.channel_id, &s, "", false)
        //         .expect("Failed to send message!");
        // } else if message.content.starts_with("!topaz search")
        //     || message.content.starts_with("!topaz analyze")
        // {
        //     if let Some(ref mut board) = self.board {
        //         let res = match board {
        //             TakGame::Standard5(board) => format!("{}", analyze_pos::<Weights5>(board)),
        //             TakGame::Standard6(board) => format!("{}", analyze_pos::<Weights6>(board)),
        //             _ => todo!(),
        //         };
        //         discord
        //             .send_message(message.channel_id, &format!("{}", res), "", false)
        //             .unwrap();
        //     } else {
        //         discord
        //             .send_message(
        //                 message.channel_id,
        //                 "Sorry I don't know the board state right now.",
        //                 "",
        //                 false,
        //             )
        //             .unwrap();
        //     }
        // } else {
        //     if let Some(ref mut board) = self.board {
        //         if let Some(m) =
        //             super::parse_move(&message.content, get_size(board), board.side_to_move())
        //         {
        //             board.do_move(m);
        //         }
        //     }
        // }
    }
    // pub async fn make_move(&mut self, context: Context, channel: ChannelId) -> Option<()> {
    //     let turn = self.topaz_turn()? && !self.undo_request;
    //     if turn {
    //         if self.board.as_ref()?.game_result().is_some() {
    //             return Some(());
    //         }
    //         let new_board = match self.board.take().unwrap() {
    //             TakGame::Standard5(board) => TakGame::Standard5(
    //                 play_async_move::<Weights5>(board, context, channel)
    //                     .await
    //                     .expect("Failed to send message"),
    //             ),
    //             TakGame::Standard6(board) => TakGame::Standard6(
    //                 play_async_move::<Weights6>(board, context, channel)
    //                     .await
    //                     .expect("Failed to send message"),
    //             ),
    //             _ => return None,
    //         };
    //         self.board = Some(new_board);
    //     }
    //     Some(())
    // }
    pub async fn request_link(context: &Context, channel: ChannelId) -> Result<()> {
        let message = serenity::builder::CreateMessage::new().content("!tak link");
        let _ = channel.send_message(&context.http, message).await?;
        Ok(())
    }
    pub async fn request_redraw(context: &Context, channel: ChannelId) -> Result<()> {
        let message = serenity::builder::CreateMessage::new().content("!tak redraw");
        let _ = channel.send_message(&context.http, message).await?;
        Ok(())
    }
    // pub fn search_room(&mut self, messages: &[Message]) -> Option<()> {
    //     let game_link = find_link(messages)?;
    //     let mut board = handle_link(&game_link)?;
    //     let mut extra_moves = Vec::new();
    //     let size = get_size(&board);
    //     for message in messages.iter() {
    //         if message.content.starts_with(LINK_START) {
    //             break;
    //         }
    //         // The person may be discussing the move in the context of a longer sentence
    //         if message.content.split_whitespace().nth(1).is_some() {
    //             continue;
    //         }
    //         if super::parse_move(&message.content, size, board.side_to_move()).is_some() {
    //             extra_moves.push(&message.content);
    //         }
    //     }
    //     for mv in extra_moves.into_iter().rev() {
    //         let m = super::parse_move(&mv, size, board.side_to_move()).unwrap();
    //         board.do_move(m);
    //     }
    //     self.board = Some(board);
    //     Some(())
    // }
}

fn get_size(game: &TakGame) -> usize {
    match game {
        TakGame::Standard5(_) => Board5::SIZE,
        TakGame::Standard6(_) => Board6::SIZE,
        TakGame::Standard7(_) => Board7::SIZE,
        _ => 6,
    }
}

fn analyze_pos<E: Evaluator + Default>(board: &mut E::Game) -> SearchOutcome<E::Game> {
    let hashtable = HashTable::new(2 << 20);
    let mut info = SearchInfo::new(MAX_DEPTH, &hashtable).time_bank(TimeBank::flat(GOAL_TIME));
    let mut eval = E::default();
    let search_res = search(board, &mut eval, &mut info);
    search_res.unwrap()
}

fn debug_tps(game: &Option<TakGame>) -> String {
    if let Some(game) = game {
        match game {
            TakGame::Standard5(board) => format!("{:?}", board),
            TakGame::Standard6(board) => format!("{:?}", board),
            TakGame::Standard7(board) => format!("{:?}", board),
            _ => "UNK".to_string(),
        }
    } else {
        return "None".to_string();
    }
}

pub fn handle_link(game_link: &str) -> Option<TakGame> {
    let ptn = super::parse_ninja_link(&game_link).ok()?;
    let (mut game, moves) = super::parse_game(&ptn)?;
    for m in moves {
        game.do_move(m);
    }
    Some(game)
}

// fn find_link(messages: &[Message]) -> Option<String> {
//     for message in messages.iter() {
//         println!("{:?}: {}", message.author, message.content);
//         if message.content.starts_with("!tak undo") {
//             break;
//         } else if message.content.starts_with("!tak rematch") {
//             break;
//         } else if message.content.starts_with("Invalid move.") {
//             break;
//         } else if message.content.starts_with("You are not ") {
//             break;
//         } else if message.author.id == TAK_BOT_ID && message.content.starts_with(LINK_START) {
//             let link = message
//                 .content
//                 .chars()
//                 .filter(|&c| c != '<' && c != '>')
//                 .collect();
//             return Some(link);
//         }
//     }
//     None
// }

pub async fn play_async_move<E: Evaluator + Default + 'static + Send>(
    mut board: E::Game,
    context: &Context,
    channel: ChannelId,
) -> Result<E::Game>
where
    E::Game: Send + Clone,
{
    // let mut tinue_search = TinueSearch::new(board).limit(NODE_LIMIT * 5).quiet();
    let clone = board.clone();
    let best_move = if false {
        // let pv_move = tinue_search.principal_variation().into_iter().next();
        // board = tinue_search.board;
        // if let Some(mv) = pv_move {
        //     format!("{}\"", mv.to_ptn::<E::Game>())
        // } else {
        //     // Maybe it's just one ply?
        //     let road_move = find_road_move(&mut board);
        //     if let Some(mv) = road_move {
        //         mv.to_ptn::<E::Game>()
        //     } else {
        //         return Err(anyhow!("Failed getting tinue pv search / road move!"));
        //     }
        // }
        todo!()
    } else {
        let mut info = SearchInfo::new(MAX_DEPTH, &GAME_TABLE).time_bank(TimeBank::flat(GOAL_TIME));
        let mut eval = E::default();
        // if board.move_num() <= 6 {
        //     eval.add_noise();
        // }
        // tokio::task::spawn_blocking(move || thread_search(board)).await??;
        let best_move = tokio::task::spawn_blocking(move || {
            let outcome = search(&mut board, &mut eval, &mut info)
                .and_then(|x| x.best_move())
                .ok_or_else(|| anyhow!("No best move from game search!"));
            GAME_TABLE.clear();
            outcome
        })
        .await??;
        // let mv = GameMove::try_from_ptn(&best_move, &board).unwrap();
        // let rev = board.do_move(mv);
        // board.null_move();
        // let mut moves = Vec::new();
        // let tak = board.can_make_road(&mut moves, None).is_some();
        // board.rev_null_move();
        // board.reverse_move(rev);
        // if tak {
        //     format!("{}'", best_move)
        // } else {
        //     best_move
        // }
        best_move
    };
    // std::thread::sleep(Duration::from_secs(5));
    let message = serenity::builder::CreateMessage::new().content(best_move);
    let _ = channel.send_message(&context.http, message).await?;
    // discord.send_message(channel, &best_move, "", false)?;
    Ok(clone)
}
