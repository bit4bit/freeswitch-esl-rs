extern crate queues;

use std::{io};
use std::io::{Read, Write, BufReader};
use queues::{CircularBuffer, Queue, IsQueue};
use crate::data::*;

const EVENT_QUEUE_SIZE: usize = 100_000;

pub trait Connectioner: Write + Read {
}

pub struct Connection<C: Connectioner> {
    reader: BufReader<C>
}

impl<C: Connectioner> Connection<C> {
    pub fn new(connection: C) -> Self {
        let reader = BufReader::new(connection);
        
        Self {
            reader: reader
        }
    }

    fn reader(&mut self) -> &mut BufReader<impl Read> {
        &mut self.reader
    }

    fn writer(&mut self) -> &mut impl Write {
        self.reader.get_mut()
    }
}

// Implement protocol Freeswitch mod_event_socket
pub struct Client<C: Connectioner> {
    connection: Connection<C>,
    api_response: Queue<Pdu>,
    command_reply: Queue<Pdu>,
    events: CircularBuffer<Pdu>
}

impl<C: Connectioner> Client<C> {
    pub fn new(connection: Connection<C>) -> Self {
        Self{
            connection: connection,
            api_response: Queue::new(),
            command_reply: Queue::new(),
            events: CircularBuffer::new(EVENT_QUEUE_SIZE)
        }
    }

    pub fn pull_event(&mut self) -> Result<Event, PduError> {
        loop {
            self.pull_and_process_pdu();

            if let Ok(pdu) = self.events.remove() {
                let event: Event = pdu.parse()?;
                return Ok(event);
            }
        }
    }

    pub fn event(&mut self, event: &str) -> Result<(), PduError> {
        self.send_command(format_args!("event plain {}", event))?;

        self.wait_for_command_reply()?;
        
        Ok(())
    }
    
    pub fn api(&mut self, cmd: &str, arg: &str) -> Result<Pdu, PduError> {
        self.send_command(format_args!("api {} {}", cmd, arg))?;

        let pdu = self.wait_for_api_response()?;

        Ok(pdu)
    }

    pub fn auth(&mut self, pass: &str) -> Result<(), &'static str> {
        let pdu = Pdu::build(self.connection.reader()).unwrap();
        
        if pdu.header("Content-Type") == "auth/request" {
            self.send_command(format_args!("auth {}", pass)).unwrap();

            let pdu = self.wait_for_command_reply().unwrap();

            if pdu.header("Reply-Text") == "+OK accepted" {
                Ok(())
            } else {
                Err("fails to authenticate")
            }
        } else {
            Err("fails to authenticate")
        }
    }

    fn wait_for_api_response(&mut self) -> Result<Pdu, PduError> {
        loop {
            self.pull_and_process_pdu();

            if let Ok(pdu) = self.api_response.remove() {
                return Ok(pdu)
            }
        }
    }

    fn wait_for_command_reply(&mut self) -> Result<Pdu, PduError> {
        loop {
            self.pull_and_process_pdu();

            if let Ok(pdu) = self.command_reply.remove() {
                return Ok(pdu)
            }
        }
    }

    fn send_command(&mut self, cmd: std::fmt::Arguments) -> io::Result<()> {
        write!(self.connection.writer(), "{}\n\n", cmd)?;
        self.connection.writer().flush()?;
        Ok(())
    }

    fn pull_and_process_pdu(&mut self) {
        let pdu = Pdu::build(self.connection.reader()).expect("fails to read pdu");
        let content_type = pdu.header("Content-Type");

        if content_type == "api/response" {
            self.api_response.add(pdu)
                .expect("fails to add to api response");
        } else if content_type == "text/event-plain" {
            self.events.add(pdu)
                .expect("fails to add event");
        } else if content_type == "command/reply" {
            self.command_reply.add(pdu)
                .expect("fails to add pdu to command_reply");
        }
    }
}

impl Connectioner for std::net::TcpStream {
}

#[cfg(test)]
mod tests {
    use super::*;

    impl Connectioner for std::io::Cursor<Vec<u8>> {
    }
    
    #[test]
    fn it_authenticate() -> Result<(), &'static str> {
        use std::io::Cursor;
        let mut protocol = Cursor::new(vec![0; 512]);
        write!(protocol, "Content-Type: auth/request\n\n").unwrap();
        write!(protocol, "Content-Type: command/reply\nReply-Text: +OK accepted\n\n").unwrap();
        protocol.set_position(0);

        let conn = Connection::new(protocol);
        let mut client = Client::new(conn);

        client.auth("test")?;
        Ok(())
    }
    
    #[test]
    fn it_invalid_authentication() {
        use std::io::Cursor;
        let mut protocol = Cursor::new(vec![0; 512]);
        write!(protocol, "Content-Type: auth/request\n\n").unwrap();
        write!(protocol, "Content-Type: command/reply\nReply-Text: -ERR invalid\n\n").unwrap();
        protocol.set_position(0);
        let conn = Connection::new(protocol);
        let mut client = Client::new(conn);

        assert_eq!("fails to authenticate", client.auth("test").unwrap_err());
    }

    #[test]
    fn it_call_api() -> Result<(), PduError> {
        use std::io::Cursor;
        let mut protocol = Cursor::new(vec![0; 512]);
        write!(protocol, "api uptime \n\n").unwrap();
        write!(protocol,
               concat!(
                   "Content-Type: api/response\n",
                   "Content-Length: 6\n\n",
                   "999666"
               )
        ).unwrap();
        protocol.set_position(0);
        let conn = Connection::new(protocol);
        let mut client = Client::new(conn);

        let pdu = client.api("uptime", "")?;

        let response: String = pdu.parse()?;
        assert_eq!("999666", response);

        Ok(())
    }

    #[test]
    fn it_pull_event() -> Result<(), PduError> {
        use std::io::Cursor;
        let mut protocol = Cursor::new(vec![0; 512]);
        write!(protocol, "
Content-Length: 526
Content-Type: text/event-plain

Event-Name: API
Core-UUID: 2379c0b2-d1a9-465b-bdc6-ca55275e591b
FreeSWITCH-Hostname: dafa872b4e1a
FreeSWITCH-Switchname: 16.20.0.9
FreeSWITCH-IPv4: 16.20.0.9
FreeSWITCH-IPv6: %3A%3A1
Event-Date-Local: 2022-10-15%2015%3A32%3A56
Event-Date-GMT: Sat,%2015%20Oct%202022%2015%3A32%3A56%20GMT
Event-Date-Timestamp: 1665847976799920
Event-Calling-File: switch_loadable_module.c
Event-Calling-Function: switch_api_execute
Event-Calling-Line-Number: 2949
Event-Sequence: 5578
API-Command: show
API-Command-Argument: calls%20as%20json

").unwrap();
        protocol.set_position(0);

        let conn = Connection::new(protocol);
        let mut client = Client::new(conn);

        let event: Event = client.pull_event()?;

        assert_eq!("API", event.get("Event-Name").unwrap());
        assert_eq!("show", event.get("API-Command").unwrap());

        Ok(())
    }

    #[test]
    fn it_pull_event_with_urldecoded_values() -> Result<(), PduError> {
        use std::io::Cursor;
        let mut protocol = Cursor::new(vec![0; 512]);
        write!(protocol, "
Content-Length: 526
Content-Type: text/event-plain

Event-Name: API
Core-UUID: 2379c0b2-d1a9-465b-bdc6-ca55275e591b
FreeSWITCH-Hostname: dafa872b4e1a
FreeSWITCH-Switchname: 16.20.0.9
FreeSWITCH-IPv4: 16.20.0.9
FreeSWITCH-IPv6: %3A%3A1
Event-Date-Local: 2022-10-15%2015%3A32%3A56
Event-Date-GMT: Sat,%2015%20Oct%202022%2015%3A32%3A56%20GMT
Event-Date-Timestamp: 1665847976799920
Event-Calling-File: switch_loadable_module.c
Event-Calling-Function: switch_api_execute
Event-Calling-Line-Number: 2949
Event-Sequence: 5578
API-Command: show
API-Command-Argument: calls%20as%20json

").unwrap();
        protocol.set_position(0);

        let conn = Connection::new(protocol);
        let mut client = Client::new(conn);

        let event: Event = client.pull_event()?;

        assert_eq!("calls as json", event.get("API-Command-Argument").unwrap());

        Ok(())
    }
}
