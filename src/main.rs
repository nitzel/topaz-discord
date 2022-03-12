use anyhow::{anyhow, Result};
use discord::model::{ChannelId, Event, Message, PublicChannel, ServerId};
use discord::Discord;
use dotenv;
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
    static ref PTN_MOVE: Regex = Regex::new(r#"([SCsc1-8]?[A-Ha-h]\d[+-<>]?\d*['"]*)"#).unwrap();
    static ref VER_RE: Regex = Regex::new(r#"rev = "\S+""#).unwrap();
}

mod play;

const TAK_TALK: ServerId = ServerId(176389490762448897);
const CHALLENGES: ChannelId = ChannelId(892609257424453672); // TODO maybe make this automatic
const NODE_LIMIT: usize = 100_000;
static TOPAZ: &'static str = "topazbot";

fn read_dotenv() {
    for arg in env::args().skip(1) {
        let path = std::path::Path::new(&arg);
        if path.exists() {
            dotenv::from_path(path).ok();
            return;
        }
    }
    dotenv::dotenv().ok();
}

fn read_cargo_toml(s: &str) -> Option<String> {
    let path = std::path::Path::new(s);
    let data = std::fs::read_to_string(path).ok()?;
    for line in data.lines() {
        if line.starts_with("topaz-tak") {
            if let Some(ver) = VER_RE.find(line) {
                return Some(ver.as_str().to_string());
            }
        }
    }
    None
}

fn main() {
    read_dotenv();
    // Log in to Discord using a bot token from the environment
    // .env file in the root of the project should have format DISCORD_TOKEN="[TOKEN]"
    let (_, token) = env::vars()
        .find(|(k, _)| k == "DISCORD_TOKEN")
        .expect("Could not read .env file!");
    let version = env::vars()
        .find(|(k, _)| k == "CARGO_FILE")
        .and_then(|(_, v)| read_cargo_toml(&v))
        .unwrap_or_else(|| "UNKNOWN".to_string());
    let discord = Discord::from_bot_token(&token).expect("Discord login failed!");

    // Establish and use a websocket connection
    let (mut connection, _) = discord.connect().expect("Discord connection failed!");
    println!("Ready.");

    let mut matches = play::Matches::default();
    matches.update_rooms(&discord).unwrap();
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
                                println!("Searching for new rooms...");
                                // Check for new rooms
                                matches.update_rooms(&discord).unwrap();
                            }
                        }
                    }
                } else if let Some(x) = matches.matches.get_mut(&message.channel_id) {
                    if message.content.starts_with("!topaz version") {
                        discord
                            .send_message(
                                message.channel_id,
                                &format!("Version: {}", version),
                                "",
                                false,
                            )
                            .unwrap();
                    } else {
                        x.do_message(&message, &discord);
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
    let mut iter = mv_str.chars().take_while(|&c| c != '\'' && c != '"');
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
        let mut string = String::new();
        string.push(first.to_ascii_lowercase());
        for c in iter.map(|c| c.to_ascii_lowercase()) {
            string.push(c);
        }
        string
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
    let moves_text = full_ptn.split("Opening").last()?.split("{").next()?;
    for m in PTN_MOVE.captures_iter(&moves_text) {
        let mv = parse_move(&m[0], size, color)?;
        moves.push(mv);
        color = !color;
    }
    // TODO Komi
    let mut komi = 0;
    if let Some(k) = meta.get("Komi") {
        komi = match k.as_str() {
            "0" => 0,
            "0.5" => 1,
            "1" => 2,
            "1.5" => 3,
            "2" => 4,
            "2.5" => 5,
            "3" => 6,
            _ => 0,
        };
        // if let Ok(val) = k.parse::<u8>() {
        //     komi = val * 2;
        // }
    }
    if let Some(tps) = meta.get("TPS") {
        let game = TakGame::try_from_tps(tps).ok()?;
        let game = match game {
            TakGame::Standard5(b) => TakGame::Standard5(b.with_komi(komi)),
            TakGame::Standard6(b) => TakGame::Standard6(b.with_komi(komi)),
            TakGame::Standard7(b) => TakGame::Standard7(b.with_komi(komi)),
            _ => return None,
        };
        return Some((game, moves));
    }
    match size {
        5 => Some((TakGame::Standard5(Board5::new().with_komi(komi)), moves)),
        6 => Some((TakGame::Standard6(Board6::new().with_komi(komi)), moves)),
        7 => Some((TakGame::Standard7(Board7::new().with_komi(komi)), moves)),
        _ => None,
    }
}

#[cfg(test)]
mod test {
    use super::*;
    #[test]
    fn tak_tinue_marks() {
        let s1 = concat!(
            "!tinue https://ptn.ninja/NoEQhgLgpgBARAJgAwIQOhWgjAdjgXQFgAoYAZQEtp4ALCCABwGcAuAejYHMqaBXAIzQBjAPYBbN", 
            "nzFgAdpDYQwAawBCIiARLAACgBswATygAnLPAAqIhmABeajUVJ7DJhPACC-A0yabSlG1g4ADY-YAB5BigZChlOeCYAdzAGP", 
            "xIsNBgAM2CYMCwSdBgAEwAWGCgAVhIAZkyocuLq4lLMoXKAYWKakkq2yph+ZuDM-nKoHuIcNpqSmoBqEgAOTO6K4JIATlHZq", 
            "AR0pEyOrN2agB50jJK3YqwAciA&name=CoewDghgXgQiAuACAbgZ0QQQEYE9XoDYAPAxAJgAYyyA6KmgRgHYg"
        );
        let t = TinueRequest::new("", &s1);
        let ptn = t.get_ptn_string().unwrap();
        let parsed = parse_game(&ptn);
        assert!(parsed.is_some());
    }
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
        let s5 = concat!(
            "!tinue https://ptn.ninja/NoEQhgLgpgBARAJgAwIQOiQZg5uBdYAFQEsBbWOARgHYAuSgVloQf2AGVjp4ALCCAA4BnWgHpRAcy48ArgC", 
            "M0AYwD2pUbNJgAdpFEQwAawBCyiGwAKAGzABPKACdK8ALLawhC9bv2E8c2BlLNk4ALwoANjYAaVVieAQ2AHkBKC1iLQl4IQB3MAF8GDAnADN", 
            "wmAATBgqAFhhFKsVauSqQMqgq8vCAWhgoWoBRTBgAYXKhuVqxgGo6scKG6pnGkaGEcsWYIWKqzEqAHhhtg86YfrLi2vZ1mDkhgEFx3wAxIcaD", 
            "4cwD9kV73x+DuRORRA3zlP5DBj1LqUSaYHpyMpgBAAPhgmFu0N8awQx18QnqFVeDAO9SAA" 
        );
        let s6 = concat!(
            "!tinue https://ptn.ninja/NoEQhgLgpgBARAJgAwIQOiQZjQRiXAXWABUBLAW1kRwC4cB2GgVnsOAGVTp4ALCCAA4BnGgHpRAcy48ArgCM", 
            "0AYwD25UbPJgAdpFEQwAawBCyiGwAKAGzABPKACcc8AIJybQoRet37CeMWUBMAAvEzMiTmCqADY2AGlVUnh0JjYAeQEoLVItCXghAHcwAUIgA"
        );
        let s7 = concat!(
            "!tinue https://ptn.ninja/NoEQhgLgpgBARAJgAwIQOiQZjQRgXAXQFgAoYAFQEsBbWOHANgC4BWFpzTQ04AZUujwAFhAgAHAM5MA9NIDmA", 
            "oQFcARmgDGAe2rTl1MADtI0iGADWAIU0RuZAAoAbMAE8oAJxzwkADxYB2BgBOBnUQ22BHF3cEeHJNMTAALysbYjJ+RLoGcIBpbUp4fDTgAHkxK", 
            "ANKAzl4CQB3MDFbUhw0GDAcIA&name=AwDwrA7AbAnFDGCAEA3AzkgKgewA4EMAvAIWwBckoQokAmYW2gOmAGYmBGWgWg6ibBgmrVkA"
        );
        let komis = [0, 0, 0, 0, 4, 5];
        for (idx, s) in [s1, s2, s3, s4, s5, s6, s7].iter().enumerate() {
            let t = TinueRequest::new("", &s);
            let ptn = t.get_ptn_string().unwrap();
            let parsed = parse_game(&ptn);
            if s == &s7 {
                let game = &parsed.as_ref().unwrap().0;
                match game {
                    TakGame::Standard6(g) => {
                        assert_eq!(g.ply(), 3);
                    }
                    _ => assert!(false),
                }
            }
            if s == &s3 {
                if let Some((_, ref moves)) = parsed {
                    let count = moves
                        .iter()
                        .filter(|m| &m.to_ptn::<Board6>() == "Sc2")
                        .count();
                    assert_eq!(count, 3);
                }
            }
            if idx == 4 || idx == 5 {
                let game = &parsed.as_ref().unwrap().0;
                match game {
                    TakGame::Standard6(g) => {
                        assert_eq!(g.komi(), komis[idx]);
                    }
                    _ => assert!(false),
                }
            }
            assert!(parsed.is_some());
        }
    }
}
