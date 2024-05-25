use std::time;
use std::env;

use topaz_discord::{find_all_tinue, get_ptn_string, parse_game, TinueStatus};
use anyhow::{anyhow, Result};
use topaz_tak::TakGame;

#[tokio::main]
async fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    let user_input = args.join(" ");
    if user_input == "" {
        println!("User input in the format of PlaytakID or TPS required.");
        return;
    }
    println!("User input {}", user_input);
    handle_tinue_req(&user_input).await.unwrap();
}


async fn handle_tinue_req(user_input: &str) -> Result<()> {
    let start_time = time::Instant::now();
    let from_tps = TakGame::try_from_tps(
        user_input
    );
    let (game, moves) = if let Ok(board) = from_tps {
        (board, Vec::new())
    } else {
        let ptn = get_ptn_string(&user_input).await?;
        parse_game(&ptn).ok_or_else(|| anyhow!("Unable to parse game"))?
    };
    if moves.len() <= 5 {
        return Err(anyhow::anyhow!("Single tinue not supported"));
    //     // Interpret as a single position
    //     match game {
    //         TakGame::Standard5(board) => find_one_tinue(board, context, message).await?,
    //         TakGame::Standard6(board) => find_one_tinue(board, context, message).await?,
    //         TakGame::Standard7(board) => find_one_tinue(board, context, message).await?,
    //         _ => anyhow::bail!("Unsupported board size"),
    //     }
    //     return Ok(());
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
        "Sure thing! Completed in {} ms.\nTinue: {}\nRoad: {}\nTimeout: {}",
        duration.as_millis(),
        printable(tinue),
        printable(road),
        printable(timeout),
    );
    println!("{}", message_string);
    Ok(())
}