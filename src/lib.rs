extern crate queues;
extern crate urldecode;

use std::{io, str, error, fmt};
use std::io::{Read, Write, BufReader};
use queues::{CircularBuffer, Queue, IsQueue};
use std::collections::HashMap;

const EVENT_QUEUE_SIZE: usize = 100_000;

// Pdu (Packet data unit) from Freeswitch mod_event_socket.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pdu {
    header: HashMap<String, String>,
    // TODO: no pub
    pub content: Vec<u8>
}

#[derive(Debug)]
pub enum PduError {
    IOError(io::Error),
    StrError(str::Utf8Error)
}

impl fmt::Display for PduError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PduError::IOError(e) => write!(f, "{}", e),
            PduError::StrError(e) => write!(f, "{}", e)
        }
    }
}

impl error::Error for PduError {
    fn source(&self) -> Option<&(dyn error::Error + 'static)> {
        match self {
            PduError::IOError(e) => Some(e),
            PduError::StrError(e) => Some(e)
        }
    }
}

impl From<io::Error> for PduError {
    fn from(e: io::Error) -> Self {
        PduError::IOError(e)
    }
}

impl From<str::Utf8Error> for PduError {
    fn from(e: str::Utf8Error) -> Self {
        PduError::StrError(e)
    }
}

// casting to another type
pub trait FromPdu: Sized {
    type Err;

    fn from_pdu(pdu: &Pdu) -> Result<Self, Self::Err>;
}

// Get Pdu content as String
impl FromPdu for String {
    type Err = PduError;
    
    fn from_pdu(pdu: &Pdu) -> Result<Self, Self::Err> {
        let content: String = str::from_utf8(&pdu.content)?.to_string();
        Ok(content)
    }
}

type Header = HashMap<String, String>;

fn header_parse(content: String) -> Header {
    let mut header = Header::new();

    content
        .split('\n')
        .filter(|line| {
            !line.is_empty()
        })
        .for_each(|line| {
            let mut item = line.splitn(2, ':');
            let key = item.next().unwrap().trim().to_string();
            let value = item.next().unwrap().trim().to_string();

            header.insert(key, urldecode::decode(value));
        });

    header
}

impl Pdu {
    // Parse Pdu to another type.
    fn parse<F: FromPdu>(&self) -> Result<F, F::Err> {
        FromPdu::from_pdu(self)
    }

    fn get(&self, k: &str) -> String {
        match self.header.get(k) {
            Some(v) => v.to_string(),
            None => "".to_string()
        }
    }

    fn build(reader: &mut impl io::BufRead) -> Result<Pdu, PduError> {
        let mut pdu = Pdu {
            header: HashMap::new(),
            content: Vec::new()
        };

        pdu.do_parse(reader)?;
        
        Ok(pdu)
    }

    fn do_parse(&mut self, reader: &mut impl io::BufRead) -> io::Result<()> {
        self.parse_header(reader)?;
        self.parse_content(reader)?;

        Ok(())
    }

    fn get_header_content(&self, reader: &mut impl io::BufRead) -> io::Result<Vec<u8>> {
        let mut raw: Vec<u8> = Vec::new();
        let mut buf: Vec<u8> = Vec::new();

        loop {
            buf.clear();
            let readed_bytes = reader.read_until(b'\n', &mut buf)?;
            if readed_bytes == 1 && buf[0] == b'\n' {
                break;
            } else {
                raw.append(&mut buf);
            }
        }

        Ok(raw)
    }

    fn parse_header(&mut self, reader: &mut impl io::BufRead) -> io::Result<()> {
        let raw = self.get_header_content(reader)?;
        let raw_str = String::from_utf8(raw).unwrap();
        self.header = header_parse(raw_str);

        Ok(())
    }

    fn parse_content(&mut self, reader: &mut impl io::BufRead) -> io::Result<()> {
        if let Some(length) = self.header.get("Content-Length") {
            let length: usize = length.parse().unwrap();
            let mut content = vec![0u8; length];
            reader.read_exact(&mut content)?;

            self.content.append(&mut content);
        }

        Ok(())
    }
}

pub struct Connection<C: Read + Write> {
    reader: BufReader<C>
}

impl<C: Read + Write> Connection<C> {
    pub fn new(inner: C) -> Connection<C> {
        let reader = BufReader::new(inner);
        
        Self {
            reader
        }
    }

    pub fn reader(&mut self) -> &mut BufReader<C> {
        &mut self.reader
    }

    pub fn get_mut(&mut self) -> &mut C  {
        self.reader.get_mut()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Event {
    inner: Header
}

impl Event {
    pub fn get(&self, k: &str) -> Option<&String> {
        self.inner.get(k)
    }
}

impl FromPdu for Event {
    type Err = PduError;

    fn from_pdu(pdu: &Pdu) -> Result<Self, Self::Err> {
        if pdu.get("Content-Type") == "text/event-plain" {
            let content = String::from(str::from_utf8(&pdu.content)?);
            let header = header_parse(content);
            Ok(Event{inner: header})
        } else {
            panic!("invalid content-type expected text/event-plain,");
        }
    }

}

// Implement protocol Freeswitch mod_event_socket
pub struct Client<T: Write + Read> {
    connection: Connection<T>,
    api_response: Queue<Pdu>,
    command_reply: Queue<Pdu>,
    events: CircularBuffer<Pdu>
}

impl<T: Read + Write> Client<T> {
    pub fn new(connection: Connection<T>) -> Self {
        Self{
            connection,
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
        
        if pdu.header["Content-Type"] == "auth/request" {
            self.send_command(format_args!("auth {}", pass)).unwrap();

            let pdu = self.wait_for_command_reply().unwrap();

            if pdu.header["Reply-Text"] == "+OK accepted" {
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
        write!(self.connection.get_mut(), "{}\n\n", cmd)?;
        self.connection.get_mut().flush()?;
        Ok(())
    }

    fn pull_and_process_pdu(&mut self) {
        let pdu = Pdu::build(self.connection.reader()).expect("fails to read pdu");
        let content_type = match pdu.header.get("Content-Type") {
            Some(v) => v,
            None => ""
        };

        if content_type == "api/response" {
            self.api_response.add(pdu)
                .expect("fails to add to api response");
        } else if content_type == "text/event-plain" {
            self.events.add(pdu)
                .expect("fails to add event");
        } else if content_type == "command/reply" {
            self.command_reply.add(pdu)
                .expect("fails to add pdu to command_reply");
        } else {
            eprintln!("unhandle content {:?}", pdu);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_authenticate() -> Result<(), &'static str> {
        use std::io::Cursor;
        let mut protocol = Cursor::new(vec![0; 512]);
        write!(protocol, "Content-Type: auth/request\n\n").unwrap();
        write!(protocol, "Content-Type: command/reply\nReply-Text: +OK accepted\n\n").unwrap();
        protocol.set_position(0);

        let conn = Connection::new(&mut protocol);
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

        let conn = Connection::new(&mut protocol);
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
        let conn = Connection::new(&mut protocol);
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

        let conn = Connection::new(&mut protocol);
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

        let conn = Connection::new(&mut protocol);
        let mut client = Client::new(conn);

        let event: Event = client.pull_event()?;

        assert_eq!("calls as json", event.get("API-Command-Argument").unwrap());

        Ok(())
    }
}
