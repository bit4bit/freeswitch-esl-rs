extern crate freeswitch_esl_rs;
use std::net::{TcpStream};
use std::env;
use freeswitch_esl_rs::{Connection,Client,Event};

fn main() -> std::io::Result<()> {
    let args: Vec<String> = env::args().collect();
    let host = &args[1];
    let mut stream = TcpStream::connect(host)?;

    let conn = Connection::new(&mut stream);
    let mut client = Client::new(conn);

    client.auth("cloudpbx").expect("fails to authenticate");
    client.event("ALL").expect("fails enabling events");

    loop {
        let event: Event = client.pull_event().unwrap();
        println!("{:?}", event);
    }
    
    Ok(())
}
