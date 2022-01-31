use anyhow::{anyhow, Result};
use discord::model::{ChannelId, Event, Message, PublicChannel, ServerId};
use discord::Discord;
use dotenv::dotenv;
use hyper::net::HttpsConnector;
use hyper::Client;
use hyper_native_tls::NativeTlsClient;
use lazy_static::lazy_static;
use lz_str::{decompress_uri, str_to_u32_vec};
use regex::Regex;
use std::collections::HashMap;
use std::io::Read;
use std::{env, time};
use topaz_tak::board::{Board5, Board6, Board7};
use topaz_tak::generate_all_moves;
use topaz_tak::search::proof::TinueSearch;
use topaz_tak::{Color, GameMove, TakBoard, TakGame};

lazy_static! {
    static ref PTN_META: Regex = Regex::new(r#"\[(?P<Key>.*?) "(?P<Value>.*?)"\]"#).unwrap();
    static ref PTN_MOVE: Regex = Regex::new(r#"([SCsc1-8]?[A-Ha-h]\d[+-<>]?\d*)"#).unwrap();
}

mod play;

const TAK_TALK: ServerId = ServerId(176389490762448897);
const CHALLENGES: ChannelId = ChannelId(892609257424453672); // TODO maybe make this automatic
const NODE_LIMIT: usize = 100_000;
static TOPAZ: &'static str = "topazbot";

fn main() {
    dotenv().ok();
    // Log in to Discord using a bot token from the environment
    // .env file in the root of the project should have format DISCORD_TOKEN="[TOKEN]"
    let (_, token) = env::vars()
        .find(|(k, _)| k == "DISCORD_TOKEN")
        .expect("Could not read .env file!");
    let discord = Discord::from_bot_token(&token).expect("Discord login failed!");

    // Establish and use a websocket connection
    let (mut connection, _) = discord.connect().expect("Discord connection failed!");
    println!("Ready.");

    let mut matches: Vec<_> = get_matches(&discord);
    for m in matches.iter() {
        let channel_id = m.id;
        let messages = discord
            .get_messages(m.id, discord::GetMessages::MostRecent, Some(40))
            .unwrap();
        let game =
            play::search_room(m.id, &discord, true, Some(messages)).expect("Search room failed!");
        if let (Some(board), Some(topaz_turn)) = (game.board, game.topaz_turn) {
            if topaz_turn {
                play::play_async_move(board, channel_id, &discord).expect("Failed to send message");
            }
        } else {
            println!("Could not fully determine game state {}!", channel_id);
        }
    }
    loop {
        match connection.recv_event() {
            Ok(Event::MessageCreate(message)) => {
                if message.content.starts_with("!tinue") {
                    // Todo handle this error better
                    if let Err(e) = handle_tinue_req(&discord, &message) {
                        println!("{}", e);
                    }
                } else if message.channel_id == CHALLENGES {
                    if message.content.starts_with("!tak") {
                        if let Some(user) = message.mentions.into_iter().next() {
                            if user.id == play::TOPAZ_ID {
                                // Reset rooms
                                matches = get_matches(&discord);
                            }
                        }
                    }
                } else if matches
                    .iter()
                    .find(|chan| chan.id == message.channel_id)
                    .is_some()
                {
                    if message.content.starts_with("Your turn ") {
                        if let Ok(game) =
                            play::search_room(message.channel_id, &discord, true, None)
                        {
                            if let (Some(board), Some(topaz_turn)) = (game.board, game.topaz_turn) {
                                if topaz_turn {
                                    play::play_async_move(board, message.channel_id, &discord)
                                        .expect("Failed to send message");
                                }
                            } else {
                                println!(
                                    "Could not fully determine game state {}!",
                                    message.channel_id
                                );
                            }
                        }
                    } else if message.content.starts_with("!topaz position") {
                        match play::search_room(message.channel_id, &discord, false, None) {
                            Ok(game) => {
                                let s = format!("This is the position, right? \n{:?}", game.board);
                                discord
                                    .send_message(message.channel_id, &s, "", false)
                                    .expect("Failed to send message!");
                            }
                            Err(e) => {
                                println!("Position get failed: {:?}", e);
                            }
                        }
                    }
                }
                // Todo respond while still logged in
            }
            Ok(_) => {}
            Err(discord::Error::Closed(code, body)) => {
                println!("Gateway closed on us with code {:?}: {}", code, body);
                break;
            }
            Err(err) => println!("Received error: {:?}", err),
        }
    }
}

fn handle_new_match(room: &AsyncChannel, discord: &Discord) -> Result<()> {
    todo!()
}

fn get_matches(discord: &Discord) -> Vec<AsyncChannel> {
    let channels = discord
        .get_server_channels(TAK_TALK)
        .expect("Failed to get Tak server channels!");
    channels
        .iter()
        .filter_map(|c| AsyncChannel::try_new(c))
        .collect()
}

#[derive(Debug)]
struct AsyncChannel {
    id: ChannelId,
    player1: String,
    player2: String,
}

impl AsyncChannel {
    pub fn try_new(channel: &PublicChannel) -> Option<Self> {
        let mut iter = channel.name.split("-🆚-");
        let p1 = iter.next()?;
        let p2 = iter.next()?;
        if iter.next().is_some() {
            None
        } else {
            if p1 == TOPAZ || p2 == TOPAZ {
                Some(Self {
                    id: channel.id,
                    player1: p1.to_string(),
                    player2: p2.to_string(),
                })
            } else {
                None
            }
        }
    }
}

struct TinueRequest<'a> {
    sender: &'a str,
    content: &'a str,
}

impl<'a> TinueRequest<'a> {
    fn new(sender: &'a str, content: &'a str) -> Self {
        Self { sender, content }
    }

    fn get_ptn_string(&self) -> Result<String> {
        let details = self
            .content
            .split_whitespace()
            .nth(1)
            .ok_or_else(|| anyhow!("Bad query request"))?;
        get_ptn_string(details)
    }
}

fn get_ptn_string(details: &str) -> Result<String> {
    if let Ok(game_id) = details.parse::<u32>() {
        // Assume it is a playtak id
        let mut buffer = String::new();
        let ssl = NativeTlsClient::new().unwrap();
        let connector = HttpsConnector::new(ssl);
        let client = Client::with_connector(connector);
        let url = format!("https://www.playtak.com/games/{}/view", game_id);
        let mut res = client.get(&url).send()?;
        res.read_to_string(&mut buffer)?;
        Ok(buffer)
    } else {
        // See if it is a ptn.ninja link
        if let Some(substr) = details.split("ptn.ninja/").nth(1) {
            let part = substr
                .split("&name")
                .next()
                .ok_or_else(|| anyhow!("Bad ptn ninja link!"))?;
            // println!("{}", part);
            let decompressed = decompress_uri(&str_to_u32_vec(part))
                .ok_or_else(|| anyhow!("Bad ptn ninja game string"))?;
            Ok(decompressed)
        } else {
            Err(anyhow!("Unknown query format!"))
        }
    }
}

fn handle_tinue_req(discord: &Discord, message: &Message) -> Result<()> {
    let req = TinueRequest::new(&message.author.name, &message.content);
    let start_time = time::Instant::now();
    let ptn = req.get_ptn_string()?;
    let game = parse_game(&ptn).ok_or_else(|| anyhow!("Unable to parse game"))?;
    let tinue_plies = match game.0 {
        TakGame::Standard5(board) => find_all_tinue(board, &game.1),
        TakGame::Standard6(board) => find_all_tinue(board, &game.1),
        TakGame::Standard7(board) => find_all_tinue(board, &game.1),
        _ => todo!(),
    };

    let mut tinue = Vec::new();
    let mut road = Vec::new();
    let mut timeout = Vec::new();
    for ply in tinue_plies.into_iter() {
        match ply {
            TinueStatus::Tinue(_) => tinue.push(ply.to_string()),
            TinueStatus::Road(_) => road.push(ply.to_string()),
            TinueStatus::Timeout(_) => timeout.push(ply.to_string()),
        }
    }
    let printable = |vec: Vec<String>| {
        if vec.len() == 0 {
            "None".to_string()
        } else {
            vec.join(", ")
        }
    };
    let duration = time::Instant::now().duration_since(start_time);

    discord.send_message(
        message.channel_id,
        &format!(
            "Sure thing, {}! Completed in {} ms.\nTinue: {}\nRoad: {}\nTimeout: {}",
            message.author.name,
            duration.as_millis(),
            printable(tinue),
            printable(road),
            printable(timeout),
        ),
        "",
        false,
    )?;
    Ok(())
}

#[derive(Debug, Clone, Copy)]
enum TinueStatus {
    Tinue(usize),
    Road(usize),
    Timeout(usize),
}

impl std::fmt::Display for TinueStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> Result<(), std::fmt::Error> {
        let ply = match self {
            TinueStatus::Tinue(x) => x,
            TinueStatus::Road(x) => x,
            TinueStatus::Timeout(x) => x,
        };
        let color = if ply % 2 == 0 { "W" } else { "B" };
        let move_num = ply / 2;
        write!(f, "{}{}", move_num, color)
    }
}

fn find_all_tinue<T: TakBoard + std::fmt::Debug>(
    mut board: T,
    moves: &[GameMove],
) -> Vec<TinueStatus> {
    let mut vec = Vec::new();
    for (idx, mv) in moves.iter().enumerate() {
        if idx < 6 {
            board.do_move(*mv);
            continue;
        }
        let s = move_s(idx);
        let mut search = TinueSearch::new(board).limit(NODE_LIMIT).quiet();
        if let Some(is_tinue) = search.is_tinue() {
            if is_tinue {
                let road_move = find_road_move(&mut search.board);
                if road_move.is_some() {
                    vec.push(TinueStatus::Road(idx + 2));
                    println!("{}: Road", s);
                } else {
                    vec.push(TinueStatus::Tinue(idx + 2));
                    println!("{}: Tinue", s);
                }
            } else {
                println!("{}: Not Tinue", s);
            }
        } else {
            vec.push(TinueStatus::Timeout(idx + 2));
            println!("{}: Timeout", s);
            println!("Timeout TPS: {:?}", search.board);
        }
        board = search.board;
        board.do_move(*mv);
    }
    vec
}

fn find_road_move<B: TakBoard>(board: &mut B) -> Option<GameMove> {
    let mut moves = Vec::new();
    let side = board.side_to_move();
    generate_all_moves(board, &mut moves);
    let road = moves.into_iter().find(|mv| {
        let rev = board.do_move(*mv);
        let road = board.road(side);
        board.reverse_move(rev);
        road
    });
    road
}

fn move_s(idx: usize) -> String {
    let color = if idx == 0 {
        "B"
    } else if idx == 1 {
        "W"
    } else if idx % 2 == 0 {
        "W"
    } else {
        "B"
    };
    format!("{}. {}", idx / 2, color)
}

fn parse_move(mv_str: &str, size: usize, color: Color) -> Option<GameMove> {
    let mut iter = mv_str.chars();
    let first = iter.next()?;
    let mv = if first == 'S' || first == 's' {
        let mut s = String::new();
        // Lowercase everything besides this first S
        s.push(first.to_ascii_uppercase());
        while let Some(c) = iter.next() {
            s.push(c.to_ascii_lowercase());
        }
        s
    } else if first == 'C' || first == 'c' {
        let mut s = String::new();
        // We need to find out if this C is a capstone or a bad square indicator
        let second = iter.next()?;
        if second.is_ascii_alphabetic() {
            // The previous must have been a capstone, make it capitalized
            s.push(first.to_ascii_uppercase());
            s.push(second.to_ascii_lowercase());
        } else {
            // The second must be a number, meaning this should be lowercase c
            s.push(first.to_ascii_lowercase());
            s.push(second);
        }
        while let Some(c) = iter.next() {
            s.push(c.to_ascii_lowercase());
        }
        s
    } else {
        mv_str.to_ascii_lowercase()
    };
    let mv = GameMove::try_from_ptn_m(&mv, size, color)?;
    Some(mv)
}

fn parse_game(full_ptn: &str) -> Option<(TakGame, Vec<GameMove>)> {
    let mut meta = HashMap::new();
    let mut moves = Vec::new();
    let mut color = Color::White;
    for m in PTN_META.captures_iter(&full_ptn) {
        meta.insert(m["Key"].to_string(), m["Value"].to_string());
    }
    let size = meta.get("Size")?.parse().ok()?;
    for m in PTN_MOVE.captures_iter(&full_ptn.split("{").next()?) {
        let mv = parse_move(&m[0], size, color)?;
        moves.push(mv);
        color = !color;
    }
    // TODO Komi
    // if let Some(komi) = meta.get("Komi") {
    //     if komi.parse::<u32>().ok()? != 0 {
    //         return None;
    //     }
    // }
    if let Some(tps) = meta.get("TPS") {
        return Some((TakGame::try_from_tps(tps).ok()?, moves));
    }
    match size {
        5 => Some((TakGame::Standard5(Board5::new()), moves)),
        6 => Some((TakGame::Standard6(Board6::new()), moves)),
        7 => Some((TakGame::Standard7(Board7::new()), moves)),
        _ => None,
    }
}

#[cfg(test)]
mod test {
    use super::*;
    // #[test]
    // fn playtak() {
    //     let g1 = ""
    // }
    #[test]
    fn ptn_ninja() {
        let s1 = concat!(
            "!tinue https://ptn.ninja/NoEQhgLgpgBARAJgAwIQOiQRjZgHHAXWABUBLAW1jk0wC4EBmWgVmcOAGVTp4ALCCAAcAzr", 
            "QD0YgObdeAVwBGaAMYB7cmLnkwAO0hiIYANYAhFRHYAFADZgAnlABOmeGAdLeANyjbSwldssbewcEeAAzAHcoiNJ5WPYuAC8q", 
            "AHZ2AGk1UngEdgB5QW9SbUl4YQiwQUIYMGdjZwATABYYJQZWlob2qBaodvkW+XaAMRaAYQbmGDD2sNCxqFC+gGoYAFF2ham1hF", 
            "WoZgBaGEkWtamwTsOYYanhgDYYBbugA"
        );
        let s2 = concat!(
            "!tinue https://ptn.ninja/NoZQlgLgpgBARABQDYEMCeAVFBrAdAYwHsBbOAXQFgAoYAUQDcoA7CeAeSaTCdmXXOrAAwkkL5", 
            "s8AIwBWAFwAGGAGoAzPIE0ASlADOAVySs4mgLTrKNcAC9YcAOwbgAMVQQd8ACznBQlAAd3OAAmRzY-Zm4Ac3gdAHd-RwwEEHgADw", 
            "AaSUzJSSD0vIL8gHoMoKF01I9irJygyXKs1JVi1Lza7NzMiqCStq6azJ7UgDZMosbhmEkYII8NalncIA&name=CIQwlgNgngBACgV", 
            "wF5IgUxgYgIwGYBsAdLjABQBCaA5mAHa1oBOAlEA&showPTN=false&theme=MYGwhgzhCWxA"
        );
        let s3 = concat!(
            "!tinue https://ptn.ninja/NoEQhgLgpgBARAJgAwIQOiQRjZgzHAXWABUBLAW1jkwA4AuXXOgVgHZDgBlU6eACwgQADgGc6AenE", 
            "BzHnwCuAIzQBjAPblx88mAB2kcRDABrAEKqIHAAoAbMAE8oAJ0zwwAKwAepK7YeOE8DoAjk6qOhzcAF5UAGwcANLqpPAIHADyQlA6pD",
            "pS8CIA7mBCHOC8iCjoWGgIACwcZJTweHRItQw0HABKUCJy1hDwSAC0AGKEMACCLmAxMADCzDAKS-O1y+vzuDDKASbbCgGT2-MAJtsmc",
            "6cB83MKLvNQAVDbAKIIADw7CADUMJy7GAAMxOvxg5y+CGuf0BIACpxcAJcTwWmAAfMD1qMllAlq8XKd1q91rjhjB9mSEMpcH8oTSYGAl",
            "pwQAdMLSETCXCAZsw-rhrh9MAEqbVhpgXPzahjcLsfkKYCyvspRTBmFBaoL1ucFSrrjrwWCQCr9gqApxCQqloSybiYLVzj9UDBcIc-vc",
            "YCJAU8vhydrgvrVlWTJqtPv9YQTcGTzdswEco3brminSy-nrJWKAkCJUmDXantLTswvsTwesYkDfpglpWYCNRkA",
        );
        let s4 = concat!(
            "!tinue https://ptn.ninja/NoEQhgLgpgBARAJgAwIQOiQRjQgzHAXWABUBLAW1jkwE4AuAFgDY6EHDgBlU6eACwgQADgGc6AenEBzHn",
            "wCuAIzQBjAPblx88mAB2kcRDABrAEKqIHAAoAbMAE8oAJ0zwAslD6OA4qWvWRVrYOjghuumDEHNwAXlRMHADS6qTwCBwA8kJQOqQ6UvAiAO", 
            "5gQoQwAIIuCi4mocqhJrgwAMIKDC0uACahIFUMALQVoVChAKIuAGYuAGKhCAq4g+UIg1MAPC1TMJxgobMbu4PTmBudTaNNUO0g7Z0ArDCjD",
            "-eDnMrtnFBMLQwAfDALIYA9rNJpgJoIKCYADUAIe5XaYAeuE6DDWmFCqJgDBGa1QMGUDxAD2U3xA30+oWafxgZ1h1JguJaCH+uDAmF+GJgCG",
            "6G1wCgQ0IQLk4AqAA"
        );
        for s in [s1, s2, s3, s4].iter() {
            let t = TinueRequest::new("", &s);
            let ptn = t.get_ptn_string().unwrap();
            if s == &s4 {
                println!("{}", ptn);
            }
            let parsed = parse_game(&ptn);
            if s == &s3 {
                if let Some((_, ref moves)) = parsed {
                    let count = moves
                        .iter()
                        .filter(|m| &m.to_ptn::<Board6>() == "Sc2")
                        .count();
                    assert_eq!(count, 3);
                }
            }
            assert!(parsed.is_some());
        }
    }
}
