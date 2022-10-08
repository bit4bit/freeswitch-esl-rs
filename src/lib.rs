use std::{io, str, error, fmt};
use std::io::{Read, Write, BufReader};

use std::collections::HashMap;

// Pdu (Packet data unit) from Freeswitch mod_event_socket.
#[derive(Debug)]
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

impl Pdu {
    // Parse Pdu to another type.
    pub fn parse<F: FromPdu>(&self) -> Result<F, F::Err> {
        FromPdu::from_pdu(self)
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
        let parts = raw_str
            .split("\n")
            .filter(|line| {
                line.bytes().count() > 0
            })
            .map(|line| {
                let mut item = line.splitn(2, ":");
                let key = item.next().unwrap();
                let value = item.next().unwrap();

                (key.trim(), value.trim()) 
            });
        
        for (key, value) in parts {
            self.header.insert(key.to_string(), value.to_string());
        }

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

// Implement protocol Freeswitch mod_event_socket
pub struct Client<T: Write + Read> {
    connection: Connection<T>,
    event_queue: Vec<Pdu>
}

impl<T: Read + Write> Client<T> {
    pub fn new(connection: Connection<T>) -> Self {
        Self{
            connection,
            event_queue: Vec::new()
        }
    }

    pub fn pull_event(&mut self) -> Result<Pdu, PduError> {
        loop {
            let pdu = Pdu::build(self.connection.reader())?;
            
            if pdu.header["Content-Type"] == "text/event-plain" {
                return Ok(pdu);
            }
        }
    }

    pub fn event(&mut self, event: &str) -> Result<(), PduError> {
        self.send_command(format_args!("event plain {}", event))?;
        Ok(())
    }
    
    pub fn api(&mut self, cmd: &str, arg: &str) -> Result<Pdu, PduError> {
        self.send_command(format_args!("api {} {}", cmd, arg))?;

        let pdu = self.wait_for_response("api/response")?;

        Ok(pdu)
    }

    pub fn auth(&mut self, pass: &str) -> Result<(), &'static str> {
        let pdu = Pdu::build(self.connection.reader()).unwrap();
        
        if pdu.header["Content-Type"] == "auth/request" {
            self.send_command(format_args!("auth {}", pass)).unwrap();

            let pdu = Pdu::build(self.connection.reader()).unwrap();
            if pdu.header["Reply-Text"] == "+OK accepted" {
                Ok(())
            } else {
                Err("fails to authenticate")
            }
        } else {
            Err("fails to authenticate")
        }
    }

    fn wait_for_response(&mut self, content_type: &str) -> Result<Pdu, PduError> {
        loop {
            let pdu = Pdu::build(self.connection.reader())?;
            if pdu.header["Content-Type"] == content_type {
                return Ok(pdu)
            } else {
                self.event_queue.push(pdu);
            }
        }
    }

    fn send_command(&mut self, cmd: std::fmt::Arguments) -> io::Result<()> {
        self.connection.get_mut().write(format!("{}\n\n", cmd).as_bytes())?;
        self.connection.get_mut().flush()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_authenticate() -> Result<(), &'static str> {
        use std::io::Cursor;
        let mut protocol = Cursor::new(vec![0; 512]);
        protocol.write_fmt(format_args!("Content-Type: auth/request\n\n")).unwrap();
        protocol.write_fmt(format_args!("Reply-Text: +OK accepted\n\n")).unwrap();
        protocol.set_position(0);

        let conn = Connection::new(&mut protocol);
        let mut client = Client::new(conn);

        client.auth("test")?;
        Ok(())
    }

    #[test]
    fn it_call_api() -> Result<(), PduError> {
        use std::io::Cursor;
        let mut protocol = Cursor::new(vec![0; 512]);
        protocol.write_fmt(format_args!("api uptime \n\n")).unwrap();
        protocol.write_fmt(format_args!(
            concat!(
                "Content-Type: api/response\n",
                "Content-Length: 6\n\n",
                "999666"
            )
        )).unwrap();
        protocol.set_position(0);
        let conn = Connection::new(&mut protocol);
        let mut client = Client::new(conn);

        let pdu = client.api("uptime", "")?;

        let response: String = pdu.parse()?;
        assert_eq!("999666", response);

        Ok(())
    }
}
