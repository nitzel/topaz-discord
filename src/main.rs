use anyhow::{anyhow, Result};
use discord::model::{Event, Message};
use discord::Discord;
use dotenv::dotenv;
use hyper::net::HttpsConnector;
use hyper::Client;
use hyper_native_tls::NativeTlsClient;
use lz_str::{decompress_uri, str_to_u32_vec};
use std::io::Read;
use std::{env, time};
use topaz_tak::board::{Board5, Board6, Board7};
use topaz_tak::search::proof::TinueSearch;
use topaz_tak::{Color, GameMove, TakBoard, TakGame};

fn main() {
    dotenv().ok();
    // Log in to Discord using a bot token from the environment
    // .env file in the root of the project should have format DISCORD_TOKEN=[TOKEN]
    let (_, token) = env::vars()
        .find(|(k, _)| k == "DISCORD_TOKEN")
        .expect("Could not read .env file!");
    let discord = Discord::from_bot_token(&token).expect("Discord login failed!");

    // Establish and use a websocket connection
    let (mut connection, _) = discord.connect().expect("Discord connection failed!");
    println!("Ready.");
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

struct TinueRequest<'a> {
    message: &'a Message,
}

impl<'a> TinueRequest<'a> {
    fn get_ptn_string(&self) -> Result<String> {
        let details = self
            .message
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
                println!("{}", part);
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
    let req = TinueRequest { message };
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
    let mut size = None;
    let mut moves = Vec::new();
    for line in full_ptn.lines() {
        if line.starts_with("[Size") {
            size = line
                .split("\"")
                .nth(1)
                .and_then(|x| x.parse::<usize>().ok());
        } else if line.starts_with("{") {
            break; // PTN Ninja alternate lines begin
        } else if !line.starts_with("[") {
            let colors = &[Color::White, Color::Black]; // We already account for the color swap
            for mv in line
                .split_whitespace()
                .skip(1)
                .zip(colors.into_iter())
                .filter_map(|(ptn, c)| GameMove::try_from_ptn_m(ptn, size?, *c))
            {
                moves.push(mv);
            }
        }
    }
    match size? {
        5 => Some((TakGame::Standard5(Board5::new()), moves)),
        6 => Some((TakGame::Standard6(Board6::new()), moves)),
        7 => Some((TakGame::Standard7(Board7::new()), moves)),
        _ => None,
    }
}
