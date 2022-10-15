# Freeswitch ESL Rust implementation (WIP)

**caution: not thread safe**

~~~rust
extern crate freeswitch_esl_rs;
use std::net::{TcpStream};
use std::env;
use freeswitch_esl_rs::{Connection,Client,Event};

fn main() -> std::io::Result<()> {
    let args: Vec<String> = env::args().collect();
    let host = &args[1];
    let event = &args[2];
    // open stream
    let mut stream = TcpStream::connect(host)?;

    // open connection
    let conn = Connection::new(&mut stream);
    // create freeswitch esl client
    let mut client = Client::new(conn);

    // authenticate to freeswitch
    client.auth("cloudpbx").expect("fails to authenticate");
    // enable events
    client.event(event).expect("fails enabling events");

    loop {
        // poll event
        let event: Event = client.pull_event().unwrap();
        println!("{:?}", event);
    }
    
    Ok(())
}
~~~
