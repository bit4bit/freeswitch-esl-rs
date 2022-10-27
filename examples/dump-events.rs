use std::net::{TcpStream};
use std::env;
use freeswitch_esl_rs::{Connection,Client,Event};

#[derive(Debug)]
struct Stats {
    events_per_sec: u64
}

fn main() -> std::io::Result<()> {
    let args: Vec<String> = env::args().collect();
    let host = &args[1];
    let event = &args[2];
    let mut stream = TcpStream::connect(host)?;

    let conn = Connection::new(stream);
    let mut client = Client::new(conn);
    let mut stats = Stats{events_per_sec: 0};

    client.auth("cloudpbx").expect("fails to authenticate");
    client.event(event).expect("fails enabling events");

    let mut now = std::time::Instant::now();
    let notify_on_secs = std::time::Duration::from_secs(1);

    loop {
        let event: Event = client.pull_event().unwrap();
        stats.events_per_sec += 1;
        
        if now.elapsed() >= notify_on_secs {
            println!("{:?} {:?}", now.elapsed(), stats);

            now = std::time::Instant::now();
            stats.events_per_sec = 0;
        }
    }
    
    Ok(())
}
