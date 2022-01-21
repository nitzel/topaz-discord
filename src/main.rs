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
use topaz_tak::search::proof::TinueSearch;
use topaz_tak::{Color, GameMove, TakBoard, TakGame};

lazy_static! {
    static ref PTN_META: Regex = Regex::new(r#"\[(?P<Key>.*?) "(?P<Value>.*?)"\]"#).unwrap();
    static ref PTN_MOVE: Regex = Regex::new(r#"([SC1-8]?[A-Ha-h]\d[+-<>]?\d*)"#).unwrap();
}

const TAK_TALK: ServerId = ServerId(176389490762448897);

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
    let channels = discord
        .get_server_channels(TAK_TALK)
        .expect("Failed to get Tak server channels!");
    let matches: Vec<_> = channels
        .iter()
        .filter_map(|c| AsyncChannel::try_new(c))
        .collect();
    for m in matches {
        println!("{:?}", m);
    }
    loop {
        match connection.recv_event() {
            Ok(Event::MessageCreate(message)) => {
                if message.content.starts_with("!tinue") {
                    // Todo handle this error better
                    if let Err(e) = handle_tinue_req(&discord, &message) {
                        println!("{}", e);
                    }
                }
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
            Some(Self {
                id: channel.id,
                player1: p1.to_string(),
                player2: p2.to_string(),
            })
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
}

fn handle_tinue_req(discord: &Discord, message: &Message) -> Result<()> {
    let req = TinueRequest::new(&message.content, &message.author.name);
    let start_time = time::Instant::now();
    let ptn = req.get_ptn_string()?;
    let game = parse_game(&ptn).ok_or_else(|| anyhow!("Unable to parse game"))?;
    let tinue_plies = match game.0 {
        TakGame::Standard5(board) => find_all_tinue(board, &game.1),
        TakGame::Standard6(board) => find_all_tinue(board, &game.1),
        TakGame::Standard7(board) => find_all_tinue(board, &game.1),
        _ => todo!(),
    };
    // This is kind of ugly, but it's fine for the moment
    let (tinue, timeout): (Vec<_>, Vec<_>) = tinue_plies.clone().into_iter().partition(|t| {
        if let TinueStatus::Tinue(_) = t {
            true
        } else {
            false
        }
    });
    let tinue: Vec<_> = tinue.into_iter().map(|x| x.to_string()).collect();
    let timeout: Vec<_> = timeout.into_iter().map(|x| x.to_string()).collect();
    let tinue_s = if tinue.len() == 0 {
        "None".to_string()
    } else {
        tinue.join(", ")
    };
    let timeout_s = if timeout.len() == 0 {
        "None".to_string()
    } else {
        timeout.join(", ")
    };
    let duration = time::Instant::now().duration_since(start_time);

    discord.send_message(
        message.channel_id,
        &format!(
            "Sure thing, {}! Completed in {} ms.\nTinue: {}\nTimeout: {}",
            message.author.name,
            duration.as_millis(),
            tinue_s,
            timeout_s
        ),
        "",
        false,
    )?;
    Ok(())
}

#[derive(Debug, Clone, Copy)]
enum TinueStatus {
    Tinue(usize),
    Timeout(usize),
}

impl std::fmt::Display for TinueStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> Result<(), std::fmt::Error> {
        let ply = match self {
            TinueStatus::Tinue(x) => x,
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
        let mut search = TinueSearch::new(board).limit(100_000).quiet();
        if let Some(b) = search.is_tinue() {
            if b {
                vec.push(TinueStatus::Tinue(idx + 2));
                println!("{}: Tinue", s);
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

fn parse_game(full_ptn: &str) -> Option<(TakGame, Vec<GameMove>)> {
    let mut meta = HashMap::new();
    let mut moves = Vec::new();
    let mut color = Color::White;
    for m in PTN_META.captures_iter(&full_ptn) {
        meta.insert(m["Key"].to_string(), m["Value"].to_string());
    }
    let size = meta.get("Size")?.parse().ok()?;
    for m in PTN_MOVE.captures_iter(&full_ptn.split("{").next()?) {
        let mv_str = &m[0];
        let mut iter = mv_str.chars();
        let first = iter.next()?;
        let mv = if first == 'S' {
            let mut s = String::new();
            // Lowercase everything besides this first S
            s.push(first);
            while let Some(c) = iter.next() {
                s.push(c.to_ascii_lowercase());
            }
            s
        } else if first == 'C' {
            let mut s = String::new();
            // We need to find out if this C is a capstone or a bad square indicator
            let second = iter.next()?;
            if second.is_ascii_alphabetic() {
                // The previous must have been a capstone, leave it capitalized
                s.push(first);
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
            m[0].to_ascii_lowercase()
        };
        let mv = GameMove::try_from_ptn_m(&mv, size, color)?;
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
        for s in [s1, s2].iter() {
            let t = TinueRequest::new("", &s);
            let ptn = t.get_ptn_string().unwrap();
            let parsed = parse_game(&ptn);
            assert!(parsed.is_some());
        }
    }
}
