extern crate freeswitch_esl_rs;
use std::net::{TcpStream};
use std::env;
use std::collections::HashMap;
use std::time::SystemTime;
use freeswitch_esl_rs::{Connection,Client,Event};

#[derive(Debug)]
struct Stats {
    event_per_tick: u64,
    events_counts_per_sec: HashMap<String, u64>
}

fn main() -> std::io::Result<()> {
    let args: Vec<String> = env::args().collect();
    let host = &args[1];
    let event = &args[2];
    let mut stream = TcpStream::connect(host)?;

    let conn = Connection::new(stream);
    let mut client = Client::new(conn);
    let mut stats = Stats{
        event_per_tick: 0,
        events_counts_per_sec: HashMap::new()
    };

    client.auth("cloudpbx").expect("fails to authenticate");
    client.event(event).expect("fails enabling events");

    let mut now = std::time::Instant::now();
    let notify_on_secs = std::time::Duration::from_secs(1);

    loop {
        let event: Event = client.pull_event().unwrap();
        let event_name = event.get("Event-Name").unwrap().to_string();

        stats.event_per_tick += 1;
        if event_name == "CUSTOM" {
            let event_subclass = event.get("Event-Subclass").unwrap().to_string();
            let mut event_name = format!("{}:{}", event_name, event_subclass);
            if let Some(action) = event.get("CC-Action") {
                event_name = format!("{}:{}", event_name, action.to_string())
            }
            if let Some(counter) = stats.events_counts_per_sec.get_mut(&event_name) {
                *counter += 1;
            } else {
                stats.events_counts_per_sec.insert(event_name, 1);
            }
        } else {
            if let Some(counter) = stats.events_counts_per_sec.get_mut(&event_name) {
                *counter += 1;
            } else {
                stats.events_counts_per_sec.insert(event_name, 1);
            }
        }

        if now.elapsed() >= notify_on_secs {
            let stamp = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap();
            println!("{} {:?} {:?}", stamp.as_secs(), now.elapsed(), stats);

            now = std::time::Instant::now();
            stats.event_per_tick = 0;
            stats.events_counts_per_sec.clear();
        }
    }
    
    Ok(())
}
