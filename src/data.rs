extern crate urldecode;

use std::{io, str, error, fmt, string, num};
use std::collections::HashMap;

// Pdu (Packet data unit) from Freeswitch mod_event_socket.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pdu {
    inner_header: HashMap<String, String>,
    // TODO: no pub
    pub content: Vec<u8>
}

#[derive(Debug)]
pub enum PduError {
    IOError(io::Error),
    StrError(str::Utf8Error),
    StringError(string::FromUtf8Error),
    NumError(num::ParseIntError)
}

impl fmt::Display for PduError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self)
    }
}

impl error::Error for PduError {
    fn source(&self) -> Option<&(dyn error::Error + 'static)> {
        match self {
            PduError::IOError(e) => Some(e),
            PduError::StrError(e) => Some(e),
            PduError::StringError(e) => Some(e),
            PduError::NumError(e) => Some(e)
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

impl From<string::FromUtf8Error> for PduError {
    fn from(e: string::FromUtf8Error) -> Self {
        PduError::StringError(e)
    }
}

impl From<num::ParseIntError> for PduError {
    fn from(e: num::ParseIntError) -> Self {
        PduError::NumError(e)
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
    
    pub fn build(reader: &mut impl io::BufRead) -> Result<Pdu, PduError> {
        let mut pdu = Pdu {
            inner_header: HashMap::new(),
            content: Vec::new()
        };

        pdu.do_parse(reader)?;
        
        Ok(pdu)
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

    fn do_parse(&mut self, reader: &mut impl io::BufRead) -> Result<(), PduError> {
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

    fn parse_header(&mut self, reader: &mut impl io::BufRead) -> Result<(), PduError> {
        let raw = self.get_header_content(reader)?;
        let raw_str = String::from_utf8(raw)?;
        self.inner_header = header_parse(raw_str);

        Ok(())
    }

    fn parse_content(&mut self, reader: &mut impl io::BufRead) -> Result<(), PduError> {
        if let Some(length) = self.inner_header.get("Content-Length") {
            let length: usize = length.parse()?;
            let mut content = vec![0u8; length];
            reader.read_exact(&mut content)?;

            self.content.append(&mut content);
        }

        Ok(())
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
