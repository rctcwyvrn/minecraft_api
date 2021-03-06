use crossbeam_channel::{unbounded, Receiver, Sender, TryRecvError};
use rustyline::error::ReadlineError;
use rustyline::Editor;
use std::{thread, time::Duration};
use warp::Filter;
extern crate pretty_env_logger;
#[macro_use]
extern crate log;

#[tokio::main(flavor = "multi_thread", worker_threads = 8)]
async fn main() {
    pretty_env_logger::init();
    let log = warp::log("log::info");

    let (send_cmd, recv_cmd) = unbounded();
    let (send_res, recv_res) = unbounded();

    let (s, r) = (send_res.clone(), recv_cmd.clone());
    thread::spawn(move || server_manager(s, r));

    let (s, r) = (send_cmd.clone(), recv_res.clone());
    // thread::spawn(move || admin_console(send_res, recv_cmd, io::stdin().as_raw_fd()));
    thread::spawn(move || admin_console(s, r));

    let (s, r) = (send_cmd.clone(), recv_res.clone());
    let list_players = warp::get()
        .and(warp::path("list"))
        .map(move || (s.clone(), r.clone()))
        .and_then(list_players)
        .with(log);

    let (s, r) = (send_cmd.clone(), recv_res.clone());
    let send_msg = warp::path!("say" / String)
        .map(move |item| (item, s.clone(), r.clone()))
        .and_then(send_msg)
        .with(log);

    let (s, r) = (send_cmd.clone(), recv_res.clone());
    let whisper_msg = warp::path!("tell" / String / String)
        .map(move |target: String, msg: String| (target, msg, s.clone(), r.clone()))
        .and_then(whisper)
        .with(log);

    let routes = list_players.or(send_msg).or(whisper_msg);
    info!("Starting server API");
    warp::serve(routes).bind(([0, 0, 0, 0], 5000)).await;
}

async fn list_players(
    (send_cmd, recv_res): (Sender<String>, Receiver<String>),
) -> Result<impl warp::Reply, warp::Rejection> {
    if let Err(e) = send_cmd.send("/list uuids".to_string()) {
        error!("send_cmd channel broken | {:?}", e);
        panic!();
    }

    let res = recv_res.recv_timeout(Duration::from_secs(5));
    match res {
        Ok(mut r) => {
            nom(&mut r);
            Ok(warp::reply::json(&r))
        }
        Err(e) => {
            let err_str = format!("{:?}", e);
            Ok(warp::reply::json(&err_str))
        }
    }
}

async fn send_msg(
    (msg, send_cmd, recv_res): (String, Sender<String>, Receiver<String>),
) -> Result<impl warp::Reply, warp::Rejection> {
    let res = urlencoding::decode(&msg);
    if let Err(_) = res {
        return Err(warp::reject::reject());
    }

    let msg = res.unwrap();
    if msg.contains("\n") {
        return Err(warp::reject::reject());
    }

    if let Err(e) = send_cmd.send(format!("/say [API] {}", msg)) {
        error!("send_cmd channel broken | {:?}", e);
        panic!();
    }

    let res = recv_res.recv_timeout(Duration::from_secs(5));
    match res {
        Ok(_) => Ok(warp::reply::json(&format!("Sent message: '{}'", msg))),
        Err(e) => {
            let err_str = format!("{:?}", e);
            Ok(warp::reply::json(&err_str))
        }
    }
}

fn nom(s: &mut String) {
    let server_str = "[minecraft/DedicatedServer]: ";
    let offset = s.find(server_str);
    if let Some(idx) = offset {
        s.replace_range(..(idx + server_str.len()), "");
    }
}

async fn whisper(
    (target, msg, send_cmd, recv_res): (String, String, Sender<String>, Receiver<String>),
) -> Result<impl warp::Reply, warp::Rejection> {
    let res = urlencoding::decode(&msg);
    if let Err(_) = res {
        return Err(warp::reject::reject());
    }

    let msg = res.unwrap();
    if msg.contains("\n") {
        return Err(warp::reject::reject());
    }

    if let Err(e) = send_cmd.send(format!("/tell {} {}", target, msg)) {
        error!("send_cmd channel broken | {:?}", e);
        panic!();
    }

    let res = recv_res.recv_timeout(Duration::from_secs(5));
    match res {
        Ok(mut r) => {
            nom(&mut r);
            Ok(warp::reply::json(&format!("{}", r)))
        }
        Err(e) => {
            let err_str = format!("{:?}", e);
            Ok(warp::reply::json(&err_str))
        }
    }
}

fn admin_console(send_cmd: Sender<String>, recv_res: Receiver<String>) {
    let mut rl = Editor::<()>::new();
    loop {
        let readline = rl.readline(">> ");
        match readline {
            Ok(line) => {
                rl.add_history_entry(line.as_str());
                let _ = send_cmd.send(line);
                thread::sleep(Duration::from_secs(1));
                let _ = recv_res.recv_timeout(Duration::from_secs(5));
            }
            Err(ReadlineError::Interrupted) => {
                error!("Killing shit");
                let _ = send_cmd.send("<KILL>".to_string());
                return;
            }
            Err(err) => {
                error!("Error: {:?}", err);
            }
        }
    }
}

fn server_manager(send_res: Sender<String>, recv_cmd: Receiver<String>) {
    info!("Starting server");

    let server_child = rexpect::spawn("/home/server/start_server.sh", Some(2000));
    // let server_child = rexpect::spawn("cat", Some(2000));
    if let Err(e) = server_child {
        error!("Failed to spawn server, exiting | {:?}", e);
        panic!();
    }
    let mut server_child = server_child.unwrap();

    let send_ch = |msg: String| {
        let res = send_res.send(msg);
        if let Err(e) = res {
            error!("Send_res channel broken | {:?}", e);
            panic!();
        }
    };

    loop {
        // Flush
        let mut c = server_child.try_read();
        while let Some(_) = c {
            print!("{}", c.unwrap());
            c = server_child.try_read();
        }

        let cmd = recv_cmd.try_recv();
        match cmd {
            Ok(_) => {}
            Err(TryRecvError::Empty) => {
                thread::sleep(Duration::from_secs(1));
                continue;
            }
            Err(TryRecvError::Disconnected) => {
                error!("Recv_cmd channel broken");
                panic!();
            }
        }
        let cmd = cmd.unwrap();
        if cmd.eq("<KILL>") {
            error!("Killing server process");
            let _ = server_child.send_control('c');
            thread::sleep(Duration::from_secs(10));
            panic!("Stopping!");
        }

        info!("Executing '{}'", cmd);
        if let Err(e) = server_child.send_line(&cmd) {
            send_ch(format!("Got error when sending to server: {:?}", e));
            continue;
        }

        let res = server_child.read_line();
        info!("Returning '{:?}'", res);
        match res {
            Ok(res) => send_ch(res),
            Err(e) => send_ch(format!("Got error when receiving from server: {:?}", e)),
        };
    }
}
