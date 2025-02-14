mod bridge;
mod ws;
mod utils;
mod config;
mod args;
mod backup;
use utils::{
    Clients, 
    send_to_clients
};
use config::{
    Session, 
    Config
};
use bridge::{
    gen_pipe, 
    set_lines, 
    update_messages, 
    replace_formatting
};
use std::{
    collections::HashMap, 
    convert::Infallible, 
    env, 
    sync::Arc, 
    time::{
        Duration, 
        Instant
    }
};
use args::parse_args;
use ws::ws_handler;
use backup::backup;
use tokio::sync::Mutex;
use warp::Filter;

lazy_static::lazy_static! {
    static ref ARGS: Vec<String> = env::args().collect();
    static ref PATH: String = ARGS[0].to_owned()[..ARGS[0].len() - 6].to_string();
    static ref SESSIONS: Vec<Session> = Config::load_sessions(PATH.to_owned());
}

#[tokio::main(flavor = "multi_thread", worker_threads = 8)]
async fn main() {
    let startup = Instant::now();

    let config = Config::load_config(PATH.to_owned());

    if ARGS.len() > 1 {
        parse_args(ARGS.to_vec());
    }

    env::set_var("LUPUS_SESSIONS", SESSIONS.len().to_string());

    let clients: Clients = Arc::new(Mutex::new(HashMap::new()));
    let ws_route = warp::path("taurus")
        .and(warp::ws())
        .and(with_clients(clients.clone()))
        .and_then(ws_handler);
    let routes = ws_route.with(warp::cors().allow_any_origin());

    let mut ip = [0; 4];
    for (i, e) in config.ws_ip.to_owned().split(".").enumerate() {
        ip[i] = match e.parse::<u8>() {
            Ok(t) => t,
            Err(e) => {
                eprintln!("*error: \x1b[31m{}", e);
                panic!("*error: invalid ip in config file! exiting\x1b[0m");
            }
        }
    }

    let mut line_map = HashMap::new();

    for i in &SESSIONS.to_owned() {
        if i.game.is_none() {
            println!(
                "*warn: \x1b[33mno game sessions detected in {}.json, continuing anyway\x1b[0m",
                i.name
            );
            continue;
        }
        gen_pipe(i.name.to_owned(), false).await;
        tokio::time::sleep(Duration::from_millis(20)).await;
        line_map.insert(i.name.to_owned(), set_lines(i.name.to_owned()));
    }

    tokio::spawn(async move {
        let mut response = Vec::new();
        for (key, value) in line_map.iter() {
            let (msg, line_count) = update_messages(key.to_owned(), *value).await;
            if msg.is_none() {
                continue;
            }
            let msg = msg.unwrap();
            line_map
                .to_owned()
                .entry(key.clone().into())
                .and_modify(|x| *x = line_count);
            response.push(msg);
        }
        let msg = replace_formatting(response.join("\n"));
        send_to_clients(&clients.clone(), &format!("MSG {}", msg)).await;
        tokio::time::sleep(Duration::from_millis(250)).await;
    });

    let mut clock: usize = 0;

    tokio::spawn(async move {
        clock += 1;
        for i in &SESSIONS.to_owned() {
            if i.game.is_none() || i.game.to_owned().unwrap().backup_interval.is_none() {
                continue;
            }

            let e = i.game.to_owned().unwrap();

            if e.backup_interval.is_some()
                && clock % e.backup_interval.unwrap() == 0
                && clock > e.backup_interval.unwrap()
            {
                let keep_time = match e.backup_keep {
                    Some(t) => t,
                    None => usize::MAX,
                };
                if e.file_path.is_none() || e.backup_interval.is_none() {
                    continue;
                }
                let _ = backup(
                    None,
                    keep_time,
                    e.file_path.unwrap(),
                    config.backup_location.to_owned(),
                    e.backup_interval.to_owned().unwrap(),
                );
            }
        }
        tokio::time::sleep(Duration::from_millis(1000)).await;
    });

    print!(
        "*info: \x1b[32mmanager loaded in: {:#?}, ",
        startup.elapsed()
    );

    println!(
        "starting websocket server on {}:{}\x1b[0m",
        config.ws_ip, config.ws_port
    );
    warp::serve(routes).run((ip, config.ws_port as u16)).await;
}
fn with_clients(clients: Clients) -> impl Filter<Extract = (Clients,), Error = Infallible> + Clone {
    warp::any().map(move || clients.clone())
}
