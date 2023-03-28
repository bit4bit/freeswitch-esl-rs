extern crate urldecode;

use std::{io, str, error, fmt, string, num};
use std::collections::HashMap;
use std::io::{Read, BufReader};

// Pdu (Packet data unit) from Freeswitch mod_event_socket.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pdu {
    inner_header: HashMap<String, String>,
    content: Vec<u8>
}

#[derive(Debug)]
pub enum ParseError {
    IOError(io::Error),
    StrError(str::Utf8Error),
    StringError(string::FromUtf8Error),
    NumError(num::ParseIntError),
    FromPduError(FromPduError)
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self)
    }
}

impl error::Error for ParseError {
    fn source(&self) -> Option<&(dyn error::Error + 'static)> {
        match self {
            ParseError::IOError(e) => Some(e),
            ParseError::StrError(e) => Some(e),
            ParseError::StringError(e) => Some(e),
            ParseError::NumError(e) => Some(e),
            ParseError::FromPduError(e) => Some(e)
        }
    }
}

impl From<io::Error> for ParseError {
    fn from(e: io::Error) -> Self {
        ParseError::IOError(e)
    }
}

impl From<str::Utf8Error> for ParseError {
    fn from(e: str::Utf8Error) -> Self {
        ParseError::StrError(e)
    }
}

impl From<string::FromUtf8Error> for ParseError {
    fn from(e: string::FromUtf8Error) -> Self {
        ParseError::StringError(e)
    }
}

impl From<num::ParseIntError> for ParseError {
    fn from(e: num::ParseIntError) -> Self {
        ParseError::NumError(e)
    }
}

#[derive(Debug)]
pub struct FromPduError(&'static str);

impl fmt::Display for FromPduError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self.0)
    }
}

impl error::Error for FromPduError {
    fn description(&self) -> &str {
        &self.0
    }
}

// casting to another type
pub trait FromPdu: Sized {
    type Err;

    fn from_pdu(pdu: &Pdu) -> Result<Self, Self::Err>;
}

// Get Pdu content as String
impl FromPdu for String {
    type Err = ParseError;
    
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
            // TODO una libreria deberia hacer esto? pues no ome
            let key = item.next().unwrap().trim().to_string();
            let value = item.next().unwrap().trim().to_string();

            header.insert(key, urldecode::decode(value));
        });

    header
}

impl Pdu {
   pub fn header(&self, k: &str) -> String {
        match self.inner_header.get(k) {
            Some(v) => v.to_string(),
            None => "".to_string()
        }
    }

    pub fn is_empty(&self) -> bool {
        self.content.len() == 0
    }

    // Parse Pdu to another type.
    pub fn parse<F: FromPdu>(&self) -> Result<F, F::Err> {
        FromPdu::from_pdu(self)
    }

    fn get(&self, k: &str) -> String {
        match self.inner_header.get(k) {
            Some(v) => v.to_string(),
            None => "".to_string()
        }
    }
}

pub struct PduParser {
}

impl PduParser {
    pub fn parse<R: Read>(reader: &mut BufReader<R>) -> Result<Pdu, ParseError> {
        let header = Self::parse_header(reader)?;
        let content = Self::parse_content(&header, reader)?;

        let pdu = Pdu {
            inner_header: header,
            content: content
        };

        Ok(pdu)
    }

    fn parse_header(reader: &mut impl io::BufRead) -> Result<Header, ParseError> {
        let raw = Self::get_header_content(reader)?;
        let raw_str = String::from_utf8(raw)?;
        let header = header_parse(raw_str);

        Ok(header)
    }

    fn parse_content(header: &Header, reader: &mut impl io::BufRead) -> Result<Vec<u8>, ParseError> {
        if let Some(length) = header.get("Content-Length") {
            let length: usize = length.parse()?;

            let mut content = vec![0u8; length];
            reader.read_exact(&mut content)?;

            return Ok(content)
        }

        Ok(vec![0u8; 0])
    }

    fn get_header_content(reader: &mut impl io::BufRead) -> io::Result<Vec<u8>> {
        let mut raw: Vec<u8> = Vec::with_capacity(1024);
        let mut buf: Vec<u8> = Vec::with_capacity(1024);

        loop {
            buf.clear();
            let readed_bytes = reader.read_until(b'\n', &mut buf)?;

            if readed_bytes == 0 {
                // necesitamos reflejar el tamano
                // de lo contraro raw queda inicializado
                // en el tamano de la capacidad
                raw.truncate(0);
                break;
            } else if readed_bytes == 1 && buf[0] == b'\n' {
                break;
            } else {
                raw.append(&mut buf);
            }
        }

        Ok(raw)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Event {
    inner: Header,
    length: usize
}

impl Event {
    // Return value of [`k`]
    pub fn get(&self, k: &str) -> Option<&String> {
        self.inner.get(k)
    }

    // Returns the length of the event.
    pub fn len(&self) -> usize {
        self.length
    }

}

impl Into<Header> for Event {
    fn into(self) -> Header {
        self.inner.clone()
    }
}

impl FromPdu for Event {
    type Err = ParseError;

    fn from_pdu(pdu: &Pdu) -> Result<Self, Self::Err> {
        if pdu.get("Content-Type") == "text/event-plain" {
            let raw = str::from_utf8(&pdu.content)?;
            let length = raw.len();
            let content = String::from(raw);
            let header = header_parse(content);
            Ok(Event{inner: header, length: length})
        } else {
            Err(ParseError::FromPduError(FromPduError("invalid content-type expected text/event-plain")))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_event_into_hashmap() -> Result<(), &'static str> {
        let mut header = Header::new();
        header.insert("Event-Name".to_string(), "TEST".to_string());
        let event = Event{inner: header.clone(), length: 99};

        let new_header: Header = event.into();
        assert_eq!(header, new_header);
        Ok(())
    }
}
