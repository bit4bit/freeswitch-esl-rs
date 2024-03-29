extern crate queues;

use std::{io, fmt, error};
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

#[derive(Debug)]
pub enum ClientError {
    ConnectionClose,
    IOError(io::Error),
    ParseError(ParseError)
}

impl fmt::Display for ClientError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self)
    }
}

impl error::Error for ClientError {
    fn source(&self) -> Option<&(dyn error::Error + 'static)> {
        match self {
            ClientError::ParseError(e) => Some(e),
            ClientError::IOError(e) => Some(e),
            ClientError::ConnectionClose => Some(self)
        }
    }
}

impl From<ParseError> for ClientError {
    fn from(e: ParseError) -> Self {
        ClientError::ParseError(e)
    }
}

impl From<io::Error> for ClientError {
    fn from(e: io::Error) -> Self {
        ClientError::IOError(e)
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

    pub fn pull_event(&mut self) -> Result<Event, ClientError> {
        loop {
            self.pull_and_process_pdu()?;

            if let Ok(pdu) = self.events.remove() {
                let event: Event = pdu.parse()?;
                return Ok(event);
            }
        }
    }

    pub fn event(&mut self, event: &str) -> Result<(), ClientError> {
        self.send_command(format_args!("event plain {}", event))?;

        self.wait_for_command_reply()?;
        
        Ok(())
    }
    
    pub fn api(&mut self, cmd: &str, arg: &str) -> Result<String, ClientError> {
        self.send_command(format_args!("api {} {}", cmd, arg))?;

        Ok(self.wait_for_api_response()?.parse()?)
    }

    pub fn auth(&mut self, pass: &str) -> Result<(), &'static str> {
        let pdu = PduParser::parse(self.connection.reader()).unwrap();
        
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

    fn wait_for_api_response(&mut self) -> Result<Pdu, ClientError> {
        loop {
            self.pull_and_process_pdu()?;

            if let Ok(pdu) = self.api_response.remove() {
                return Ok(pdu)
            }
        }
    }

    fn wait_for_command_reply(&mut self) -> Result<Pdu, ClientError> {
        loop {
            self.pull_and_process_pdu()?;

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

    fn pull_and_process_pdu(&mut self) -> Result<(), ClientError> {
        let pdu = PduParser::parse(self.connection.reader()).expect("fails to read pdu");
        let content_type = pdu.header("Content-Type");

        if content_type == "api/response" {
            self.api_response.add(pdu)
                .expect("fails to add to api response");
            Ok(())
        } else if content_type == "text/disconnect-notice" {
            Err(ClientError::ConnectionClose)
        } else if content_type == "text/event-plain" {
            self.events.add(pdu)
                .expect("fails to add event");

            Ok(())
        } else if content_type == "command/reply" {
            self.command_reply.add(pdu)
                .expect("fails to add pdu to command_reply");
            Ok(())
        } else if pdu.is_empty() {
            Err(ClientError::ConnectionClose)
        } else {
            panic!("missing handler for {:?}", pdu);
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
    fn it_call_api() -> Result<(), ClientError> {
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

        let response: String = pdu.parse().unwrap();
        assert_eq!("999666", response);

        Ok(())
    }

    #[test]
    fn it_pull_event() -> Result<(), ClientError> {
        use std::io::Cursor;
        let mut protocol = Cursor::new(vec![0; 512]);
        write!(protocol, "Content-Length: 526
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

        let event: Event = client.pull_event().unwrap();

        assert_eq!("API", event.get("Event-Name").unwrap());
        assert_eq!("show", event.get("API-Command").unwrap());

        Ok(())
    }

    #[test]
    fn it_pull_event_with_urldecoded_values() -> Result<(), ClientError> {
        use std::io::Cursor;
        let mut protocol = Cursor::new(vec![0; 512]);
        write!(protocol, "Content-Length: 526
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

    #[test]
    fn it_handle_event_disconnection_from_server() {
        use std::io::Cursor;
        let mut protocol = Cursor::new(vec![0; 512]);
        write!(protocol, "Content-Type: text/disconnect-notice
Content-Length: 67

Disconnected, goodbye.
See you at ClueCon! http://www.cluecon.com/").unwrap();
        protocol.set_position(0);

        let conn = Connection::new(protocol);
        let mut client = Client::new(conn);

        let event = client.pull_event();
        assert_eq!(true, event.is_err());
    }

    #[test]
    fn it_handle_socket_disconnection_from_server() {
        use std::io::Cursor;
        let mut protocol = Cursor::new(vec![0; 512]);
        write!(protocol, "").unwrap();
        protocol.set_position(0);

        let conn = Connection::new(protocol);
        let mut client = Client::new(conn);

        let event = client.pull_event();
        assert_eq!(true, event.is_err());
    }
}
