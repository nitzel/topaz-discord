use anyhow::{anyhow, Result};
use dotenv;
use hyper::client::HttpConnector;
// use hyper::net::HttpsConnector;
// use hyper::Client;
use lazy_static::lazy_static;
use lz_str::{decompress_uri, str_to_u32_vec};
use regex::Regex;
use std::collections::HashMap;
use std::io::{BufRead, Write};
use std::{env, time};
use topaz_tak::board::{Board5, Board6, Board7};
use topaz_tak::search::proof::{InteractiveSearch, TinueSearch};
use topaz_tak::{generate_all_moves, Position};
use topaz_tak::{Color, GameMove, TakBoard, TakGame};

use serenity::model::channel::ReactionType;
use serenity::model::prelude::*;
use serenity::prelude::*;

use hyper_rustls::HttpsConnector;

lazy_static! {
    static ref HTTP_CLIENT: hyper::Client<HttpsConnector<HttpConnector>> = {
        let https = hyper_rustls::HttpsConnectorBuilder::new()
            .with_native_roots()
            .https_only()
            .enable_http1()
            .build();

        let client: hyper::Client<_, hyper::Body> = hyper::Client::builder().build(https);
        client
    };
}

#[derive(Debug)]
struct Handler;

#[serenity::async_trait]
impl EventHandler for Handler {
    async fn message(&self, context: Context, msg: Message) {
        if msg.content.starts_with("!tinue") {
            tracing::debug!("Running Tinue...");
            react(&context, &msg, "👍").await;
            if let Err(e) = handle_tinue_req(&context, &msg).await {
                tracing::warn!("Error handling tinue request: {}", e);
                react(&context, &msg, "❌").await;
            }
        }
        if msg.content == "!ping" {
            tracing::debug!("Should send pong...");
            if let Err(e) = msg.channel_id.say(&context, "Pong!").await {
                tracing::warn!("Failed to send message: {:?}", e);
            }
        }
    }
    async fn ready(&self, _: Context, ready: Ready) {
        tracing::debug!("{} is connected!", ready.user.name);
    }
}

async fn react(context: &Context, msg: &Message, unicode: &str) {
    if let Err(_) = msg
        .react(&context, ReactionType::Unicode(unicode.to_string()))
        .await
    {
        tracing::warn!("Failed to send reaction: {}", unicode);
    }
}

fn main() {
    dotenv::dotenv().expect("Failed to load .env file");
    tokio::runtime::Builder::new_current_thread()
        .max_blocking_threads(1)
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let subscriber = tracing_subscriber::FmtSubscriber::builder()
                // all spans/events with a level higher than TRACE (e.g, debug, info, warn, etc.)
                // will be written to stdout.
                .with_max_level(tracing::Level::DEBUG)
                .finish();
            tracing::subscriber::set_global_default(subscriber)
                .expect("setting default subscriber failed");
            let token = env::var("DISCORD_TOKEN").expect("Expected a token in the environment");

            let mut client = Client::builder(
                &token,
                GatewayIntents::from_bits(1 << 15 | 101376).unwrap()
                    | GatewayIntents::non_privileged(),
            )
            .event_handler(Handler)
            .await
            .unwrap();

            client.start().await.unwrap();
        });
}

lazy_static! {
    static ref PTN_META: Regex = Regex::new(r#"\[(?P<Key>.*?) "(?P<Value>.*?)"\]"#).unwrap();
    static ref PTN_MOVE: Regex = Regex::new(r#"([SCsc1-8]?[A-Ha-h]\d[+-<>]?\d*['"]*)"#).unwrap();
    static ref VER_RE: Regex = Regex::new(r#"rev = "\S+""#).unwrap();
}

// mod play;

// const TAK_TALK: ServerId = ServerId(176389490762448897);
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

// fn main() {
//     // let mut matches = play::Matches::default();
//     // matches.update_rooms(&discord).unwrap();
//     // loop {
//     //     match connection.recv_event() {
//     //         Ok(Event::MessageCreate(message)) => {
//     //             if message.content.starts_with("!tinue") {
//     //                 // Todo handle this error better
//     //                 if let Err(e) = handle_tinue_req(&discord, &message) {
//     //                     println!("{}", e);
//     //                 }
//     //             } else if let Some(x) = matches.matches.get_mut(&message.channel_id) {
//     //                 if message.content.starts_with("!topaz version") {
//     //                     discord
//     //                         .send_message(
//     //                             message.channel_id,
//     //                             &format!("Version: {}", version),
//     //                             "",
//     //                             false,
//     //                         )
//     //                         .unwrap();
//     //                 } else {
//     //                     x.do_message(&message, &discord);
//     //                 }
//     //             }
//     //             // Todo respond while still logged in
//     //         }
//     //         Ok(Event::ChannelCreate(ch)) | Ok(Event::ChannelUpdate(ch)) => {
//     //             if let discord::model::Channel::Public(ref ch) = ch {
//     //                 if ch.name.contains(TOPAZ) {
//     //                     if let Some(game) = crate::play::AsyncGameState::try_new(ch) {
//     //                         matches.track_room(&discord, game).unwrap();
//     //                     } else {
//     //                         matches.untrack_room(&discord, ch).unwrap();
//     //                     }
//     //                 }
//     //             }
//     //         }
//     //         Ok(_) => {}
//     //         Err(discord::Error::Closed(code, body)) => {
//     //             println!("Gateway closed on us with code {:?}: {}", code, body);
//     //             break;
//     //         }
//     //         Err(err) => println!("Received error: {:?}", err),
//     //     }
//     // }
// }

struct TinueRequest<'a> {
    sender: &'a str,
    content: &'a str,
}

impl<'a> TinueRequest<'a> {
    fn new(sender: &'a str, content: &'a str) -> Self {
        Self { sender, content }
    }

    async fn get_ptn_string(&self) -> Result<String> {
        let details = self
            .content
            .split_whitespace()
            .nth(1)
            .ok_or_else(|| anyhow!("Bad query request"))?;
        get_ptn_string(details).await
    }
}

async fn get_ptn_string(details: &str) -> Result<String> {
    use std::io::Write;
    if let Ok(game_id) = details.parse::<u32>() {
        // Assume it is a playtak id
        let mut buffer = Vec::new();
        // let ssl = NativeTlsClient::new().unwrap();
        // let connector = HttpsConnector::new(ssl);
        // let client = Client::with_connector(connector);
        let url = format!("https://playtak.com/games/{}/view", game_id).parse()?;
        let mut res = HTTP_CLIENT.get(url).await?;
        while let Some(chunk) = hyper::body::HttpBody::data(&mut res.body_mut()).await {
            buffer.write_all(&chunk?)?;
        }
        // res.read_to_string(&mut buffer)?;
        Ok(String::from_utf8(buffer)?)
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

async fn handle_tinue_req(context: &serenity::client::Context, message: &Message) -> Result<()> {
    let req = TinueRequest::new(&message.author.name, &message.content);
    let start_time = time::Instant::now();
    let ptn = req.get_ptn_string().await?;
    let (game, moves) = parse_game(&ptn).ok_or_else(|| anyhow!("Unable to parse game"))?;
    if moves.len() <= 5 {
        // Interpret as a single position
        match game {
            TakGame::Standard5(board) => find_one_tinue(board, context, message).await?,
            TakGame::Standard6(board) => find_one_tinue(board, context, message).await?,
            TakGame::Standard7(board) => find_one_tinue(board, context, message).await?,
            _ => anyhow::bail!("Unsupported board size"),
        }
        return Ok(());
    }
    let tinue_plies = match game {
        TakGame::Standard5(board) => find_all_tinue(board, &moves),
        TakGame::Standard6(board) => find_all_tinue(board, &moves),
        TakGame::Standard7(board) => find_all_tinue(board, &moves),
        _ => anyhow::bail!("Unsupported board size"),
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
    let message_string = format!(
        "Sure thing, {}! Completed in {} ms.\nTinue: {}\nRoad: {}\nTimeout: {}",
        message.author.name,
        duration.as_millis(),
        printable(tinue),
        printable(road),
        printable(timeout),
    );
    message
        .channel_id
        .say(context.http.clone(), message_string)
        .await?;
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

fn thread_search<T: TakBoard + std::fmt::Debug>(board: T) -> Result<Option<bool>> {
    let mut search = TinueSearch::new(board).limit(NODE_LIMIT).quiet();
    let tinue = search.is_tinue();
    if search.aborted() {
        tracing::debug!("Aborting search on: {:?}", search.board);
        // Todo
        return Ok(None);
    }
    let tinue = tinue.ok_or_else(|| anyhow!("Failed to execute Tinue check"))?;
    tracing::debug!("Valid Tinue Result Received: {:?}", search.board);
    // Build temp file

    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open("proof-data.txt")?;
    let mut file = std::io::BufWriter::new(file);
    let mut hist = Vec::new();
    let mut zobrist_hist = std::collections::HashSet::new();
    search.rebuild(
        &mut file,
        &mut hist,
        &mut zobrist_hist,
        search.is_attacker(),
    )?;
    file.flush()?;
    // Build svg
    let reader = std::io::BufReader::new(std::fs::File::open("proof-data.txt").unwrap());
    let writer = std::io::BufWriter::new(
        std::fs::OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open("tinue.svg")?,
    );
    let reader = std::io::BufReader::new(std::fs::File::open("proof-data.txt").unwrap());
    inferno::flamegraph::handle_file(reader, writer)?;
    tracing::debug!("Handled file!");
    Ok(Some(tinue))
}

async fn find_one_tinue<T: TakBoard + std::fmt::Debug + Send + 'static>(
    board: T,
    context: &Context,
    message: &Message,
) -> Result<()> {
    // let tinue = thread_search(board.clone());
    let tinue = tokio::task::spawn_blocking(move || thread_search(board)).await??;
    if let Some(tinue) = tinue {
        let f1 = tokio::fs::OpenOptions::new()
            .read(true)
            .open("tinue.svg")
            .await?;
        let files = vec![(&f1, "tinue.svg")];
        let st = if tinue {
            "Tinue Found!"
        } else {
            "No Tinue Found."
        };
        message
            .channel_id
            .send_files(context, files, |m| {
                m.content(format!(
                    "{}\n{}",
                    st, "Open this file in a web browser for best results."
                ))
            })
            .await?;
    } else {
        message.channel_id.say(context, "Timed out. Sorry.").await?;
    }
    // messag
    // if let Some(is_tinue) = search.is_tinue() {
    //     if is_tinue {
    //         let road_move = find_road_move(&mut search.board);
    //         if road_move.is_some() {
    //             println!("{}: Road", s);
    //             return Some(TinueStatus::Road(ply_index));
    //         } else {
    //             println!("{}: Tinue", s);
    //             return Some(TinueStatus::Tinue(ply_index));
    //         }
    //     } else {
    //         println!("{}: Not Tinue", s);
    //         return None;
    //     }
    // } else {
    //     println!("{}: Timeout", s);
    //     println!("Timeout TPS: {:?}", search.board);
    //     return Some(TinueStatus::Timeout(ply_index));
    // }
    Ok(())
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
                    tracing::debug!("{}: Road", s);
                } else {
                    vec.push(TinueStatus::Tinue(idx + 2));
                    tracing::debug!("{}: Tinue", s);
                }
            } else {
                tracing::debug!("{}: Not Tinue", s);
            }
        } else {
            vec.push(TinueStatus::Timeout(idx + 2));
            tracing::debug!("{}: Timeout\nTimeout TPS {:?}", s, search.board);
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
        tracing::debug!("TPS: {}", tps);
        let game = TakGame::try_from_tps(tps).ok()?;
        let game = match game {
            TakGame::Standard5(b) => TakGame::Standard5(b.with_komi(komi)),
            TakGame::Standard6(b) => TakGame::Standard6(b.with_komi(komi)),
            TakGame::Standard7(b) => TakGame::Standard7(b.with_komi(komi)),
            _ => return None,
        };
        if let Color::Black = game.side_to_move() {
            for i in 0..moves.len() {
                let mv = moves[i];
                if mv.is_place_move() {
                    moves[i] =
                        GameMove::from_placement(mv.place_piece().swap_color(), mv.src_index());
                }
            }
        }
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
    fn start_from_tps_black() {
        let s1 = concat!(
            "https://ptn.ninja/NoFQCgygBARATAGgIwMUpa5IggHgehRThKwGFlUc581UTsrkNGC7F3DVn04L1KSWtw4YcRKHCgAWOD", 
            "AC6wACIBDAC4BTWHAAMJAHQ7p+kgtABLALZaYOpAC4AzADZ7SHWYjnNsABZq1AAcAZ3t8fABzb18AVwAjfQBjAHtLfFjLFQA7", 
            "dXw1FQBrACFktTMwABsVAE8NACckWBBkwJUALxKyxUqa+qkYFQrzZKy9T3M2m2czAGlU820zAHlAjSzzLIjYYIB3FUCFKEcAE2", 
            "cAWhlE6QuAM2cjjWkAPigro5vpAGoMV+cAHkkGn+AHIjsckE8fsc4J9QcdrlB4aC4MdHJ8gA"
        );
        let tps = "2,12,x,21S,1,221S/1,1,22221C,1112,2S,21/2,1,2,112S,1,x/2,1,22221S,2,1,2/1,2,11112C,1,1,1/2,2,2,x,12,112S 1 49";
        let ptn = get_ptn_string(s1).unwrap();
        let (mut game, moves) = parse_game(&ptn).unwrap();
        for mv in moves {
            game.do_move(mv);
        }
        match game {
            TakGame::Standard6(board) => {
                let s = format!("{:?}", board);
                assert_eq!(tps, s);
                println!("{}", s);
            }
            _ => assert!(false),
        }
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
