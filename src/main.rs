use warp::{Filter};
use std::thread;
use crossbeam_channel::{Sender, Receiver, unbounded};
use std::time::Duration;

#[tokio::main(flavor = "multi_thread", worker_threads = 8)]
async fn main() {
    let (send_cmd, recv_cmd) = unbounded();
    let (send_res, recv_res) = unbounded();
    thread::spawn(move || server_manager(send_res, recv_cmd));

    let (s,r) = (send_cmd.clone(), recv_res.clone());
    let list_players = warp::get()
        .and(warp::path("list"))
        .map(move || (s.clone(), r.clone()))
        .and_then(list_players);
    
    let (s,r) = (send_cmd.clone(), recv_res.clone());
    let send_msg = warp::path!("say" / String)
        .map(move |item| (item, s.clone(), r.clone()))
        .and_then(send_msg);
    
    
    let routes = list_players.or(send_msg);
    println!("Starting server API");
    warp::serve(routes)
        .bind(([0,0,0,0], 5000))
        .await;
}

async fn list_players((send_cmd, recv_res): (Sender<String>, Receiver<String>)) ->  Result<impl warp::Reply, warp::Rejection> {
    if let Err(e) = send_cmd.send("/list uuids".to_string()) {
        panic!("send_cmd channel broken | {:?}", e);
    }

    let res = recv_res.recv_timeout(Duration::from_secs(5));
    match res {
        Ok(r) => Ok(warp::reply::json(&r)),
        Err(e) => {
            let err_str = format!("{:?}",e);
            Ok(warp::reply::json(&err_str))
        }
    }
}

async fn send_msg((msg, send_cmd, recv_res): (String, Sender<String>, Receiver<String>)) ->  Result<impl warp::Reply, warp::Rejection> {
    let res = urlencoding::decode(&msg);
    if let Err(_) = res {
        return Err(warp::reject::reject());
    }
    let msg = res.unwrap();
    
    if let Err(e) = send_cmd.send(format!("/say [API] {}", msg)) {
        panic!("send_cmd channel broken | {:?}", e);
    }

    let res = recv_res.recv_timeout(Duration::from_secs(5));
    match res {
        Ok(_) => Ok(warp::reply::json(&format!("Sent message: '{}'", msg))),
        Err(e) => {
            let err_str = format!("{:?}",e);
            Ok(warp::reply::json(&err_str))
        }
    }
}

fn server_manager(send_res: Sender<String>, recv_cmd: Receiver<String>) {
    println!("Starting server");

    let server_child = rexpect::spawn("/home/server/start_server.sh", Some(2000));
    if let Err(e) = server_child {
        panic!("Failed to spawn server, exiting | {:?}", e);
    }
    let mut server_child = server_child.unwrap();

    let send_ch = | msg: String | {
        let res = send_res.send(msg);
        if let Err(e) = res {
            panic!("Send_res channel broken | {:?}", e);
        }
    };

    loop {
        let cmd = recv_cmd.recv();
        if let Err(e) = cmd {
            panic!("Recv_cmd channel broken | {:?}", e);
        }
        let cmd = cmd.unwrap();

        // Flush
        let mut c = Some('c');
        while let Some(_) = c {
            c = server_child.try_read();
        }

        if let Err(e) = server_child.send_line(&cmd) {
            send_ch(format!("Got error when sending to server: {:?}", e));
        }

        let res = server_child.read_line();
        match res {
            Ok(res) => send_ch(res),
            Err(e) => send_ch(format!("Got error when receiving from server: {:?}", e)),
        };
    }
}
