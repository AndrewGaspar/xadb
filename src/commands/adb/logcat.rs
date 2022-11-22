use std::process::Stdio;

use bytes::{Buf, BytesMut};
use chrono::{prelude::*, DateTime};
use futures::Stream;
use quick_error::quick_error;
use tokio::io::BufReader;
use tokio_util::codec::FramedRead;

const EXPECTED_BEGINNING_OF_BUFFER: &[u8] = b"--------- beginning of ";
const EXPECTED_BEGINNING_OF_LOG_HEADER: &[u8] = b"[ ";

#[allow(unused)]
const LOG_LEVEL_UNKNOWN: u8 = 0;
#[allow(unused)]
const LOG_LEVEL_DEFAULT: u8 = 1;
const LOG_LEVEL_VERBOSE: u8 = 2;
const LOG_LEVEL_DEBUG: u8 = 3;
const LOG_LEVEL_INFO: u8 = 4;
const LOG_LEVEL_WARN: u8 = 5;
const LOG_LEVEL_ERROR: u8 = 6;
const LOG_LEVEL_FATAL: u8 = 7;
#[allow(unused)]
const LOG_LEVEL_SILENT: u8 = 8;

#[allow(unused)]
const LOG_ID_MAIN: u32 = 0;
#[allow(unused)]
const LOG_ID_RADIO: u32 = 1;
const LOG_ID_EVENTS: u32 = 2;
#[allow(unused)]
const LOG_ID_SYSTEM: u32 = 3;
#[allow(unused)]
const LOG_ID_CRASH: u32 = 4;
const LOG_ID_STATS: u32 = 5;
const LOG_ID_SECURITY: u32 = 6;
#[allow(unused)]
const LOG_ID_KERNEL: u32 = 7;

#[derive(Debug)]
pub enum LogLevel {
    Other(u8),
    Verbose,
    Debug,
    Info,
    Warning,
    Error,
    Fatal,
}

#[derive(Debug)]
pub struct LogLongMessage {
    pub timestamp: DateTime<chrono::FixedOffset>,
    pub uid: Option<String>,
    pub pid: u32,
    pub tid: u32,
    pub level: LogLevel,
    pub tag: String,
    pub message: Vec<u8>,
}

#[derive(Debug)]
pub struct TextLogBuffer {
    pub level: LogLevel,
    pub tag: String,
    pub message: String,
}

#[derive(Debug)]
pub struct BinaryLogBuffer {
    pub tag: i32,
}

#[derive(Debug)]
pub enum LogBuffer {
    TextLog(TextLogBuffer),
    BinaryLog(BinaryLogBuffer),
}

#[derive(Debug)]
pub struct LogMessage {
    pub timestamp: chrono::NaiveDateTime,
    pub pid: i32,
    pub tid: u32,
    pub lid: Option<u32>,
    pub uid: Option<u32>,
    pub buffer: LogBuffer,
}

#[derive(Debug)]
pub enum LogItem {
    LogBeginning(String),
    LogMessage(LogLongMessage),
    LogUnknown(Vec<u8>),
}

quick_error! {
    #[derive(Debug)]
    pub enum LogcatDecodeError {
        Io(err: std::io::Error) {
            from()
        }
    }
}

#[allow(unused)]
struct LogcatStringDecoder {
    is_in_error_state: bool,
    error_data: Vec<u8>,
}

#[allow(unused)]
impl LogcatStringDecoder {
    fn new() -> Self {
        Self {
            is_in_error_state: false,
            error_data: Vec::new(),
        }
    }

    // typically called when identifying an error condition
    fn scan_out_error_state(&mut self, src: &mut BytesMut) -> Option<LogItem> {
        let mut iter_bytes = src.windows(3).enumerate();
        while let Some((i, window)) = iter_bytes.next() {
            if window[0] == b'\n' && window[1] == b'\n' && (window[2] == b'-' || window[2] == b'[')
            {
                // push remaining error data into error_data
                let mut error_data = Vec::new();
                std::mem::swap(&mut error_data, &mut self.error_data);

                // append all the unknown log contents
                error_data.extend_from_slice(&src[..i]);

                // skip '\n\n' to get to next message
                src.advance(i + 2);
                self.is_in_error_state = false;

                return Some(LogItem::LogUnknown(error_data));
            }
        }

        // read a bunch more in to keep scanning for error
        self.error_data.extend_from_slice(&src[..]);
        src.advance(src.len());
        src.reserve(1024);
        return None;
    }

    fn enter_error_state(&mut self, src: &mut BytesMut) -> Option<LogItem> {
        self.is_in_error_state = true;
        return self.scan_out_error_state(src);
    }

    // check for "--------- beginning of system"
    fn decode_beginning_of_ring_buffer(&mut self, src: &mut BytesMut) -> Option<LogItem> {
        if src.len() < EXPECTED_BEGINNING_OF_BUFFER.len() {
            src.reserve(EXPECTED_BEGINNING_OF_BUFFER.len() - src.len() + 128);
            return None;
        }

        if !src[..].starts_with(EXPECTED_BEGINNING_OF_BUFFER) {
            return self.enter_error_state(src);
        }

        let start_of_ring_name = EXPECTED_BEGINNING_OF_BUFFER.len();

        let chars_i = src[start_of_ring_name..]
            .iter()
            .copied()
            .enumerate()
            .map(|(i, c)| (i + start_of_ring_name, c));
        let mut ring_name = Vec::new();
        for (i, c) in chars_i {
            // maybe not precise enough
            if c.is_ascii_alphanumeric() || c == b'_' {
                ring_name.push(c);
            } else if c == b'\n' {
                // end of message
                src.advance(i + 1);
                return Some(LogItem::LogBeginning(
                    String::from_utf8_lossy(&ring_name[..]).into_owned(),
                ));
            } else {
                return self.enter_error_state(src);
            }
        }

        src.reserve(128);
        return None;
    }

    // decode long style log state
    fn decode_log(&mut self, src: &mut BytesMut) -> Option<LogItem> {
        const DATE_FORMAT_LEN: usize = b"2022-11-04 00:50:26.234185959 +0000".len();
        const MINIMAL_LOG_LEN: usize =
            b"[ 0000-00-00 00:00:00.000000000 +0000 00000:00000:00000 V/a ]\n\n\n".len();

        // sanity check
        if src.len() < MINIMAL_LOG_LEN {
            src.reserve(1024 - MINIMAL_LOG_LEN);
            return None;
        }

        if &src[0..2] != &b"[ "[..] {
            return self.enter_error_state(src);
        }

        let mut i = b"[ ".len();

        // get timestamp as string
        let Ok(timestamp) = std::str::from_utf8(&src[i..][..DATE_FORMAT_LEN]) else {
            return self.enter_error_state(src);
        };

        // parse date
        let timestamp = match DateTime::parse_from_str(timestamp, "%Y-%m-%d %H:%M:%S.%9f %z") {
            Ok(timestamp) => timestamp,
            Err(_) => return self.enter_error_state(src),
        };

        i += DATE_FORMAT_LEN;

        // must be at least one space
        if src[i] != b' ' {
            return self.enter_error_state(src);
        }

        // skip whitespace until start of uid/pid
        while src[i] == b' ' {
            i += 1;
        }

        // parse "uid: pid: tid" or "pid: tid" and then figure out which is which

        // expect at least one target character
        if !(src[i].is_ascii_alphanumeric() || src[i] == b'_') {
            return self.enter_error_state(src);
        }

        let maybe_uid_start = i;
        while src[i].is_ascii_alphanumeric() || src[i] == b'_' {
            i += 1;
        }
        let maybe_uid_end = i;

        // whether uid or pid, this must be ':'
        if src[i] != b':' {
            return self.enter_error_state(src);
        }
        i += 1;

        // skip any whitespace
        while src[i] == b' ' {
            i += 1;
        }

        let maybe_pid_start = i;
        // definitely must be numeric since this is either pid or tid
        // expect at lesat one digit
        if !src[i].is_ascii_digit() {
            return self.enter_error_state(src);
        }

        while src[i].is_ascii_digit() {
            i += 1;
        }
        let maybe_pid_end = i;

        // if we've reached a colon, then the original bit is a uid, and we still have the tid to parse
        let (uid, pid, tid) = if src[i] == b':' {
            i += 1;

            // skip any whitespace
            while src[i] == b' ' {
                i += 1;
            }

            // parse definitely a tid
            if !src[i].is_ascii_digit() {
                return self.enter_error_state(src);
            }

            let tid_start = i;
            while src[i].is_ascii_digit() {
                i += 1;
            }
            let tid_end = i;

            let uid = maybe_uid_start..maybe_uid_end;
            let pid = maybe_pid_start..maybe_pid_end;
            let tid = tid_start..tid_end;

            // unwraps are safe because we've validated all preconditions
            (
                Some(String::from_utf8_lossy(&src[uid]).to_string()),
                std::str::from_utf8(&src[pid]).unwrap().parse().unwrap(),
                std::str::from_utf8(&src[tid]).unwrap().parse().unwrap(),
            )
        } else {
            let pid = maybe_uid_start..maybe_uid_end;

            // validate it's actually a pid
            for c in &src[pid.clone()] {
                if !c.is_ascii_digit() {
                    return self.enter_error_state(src);
                }
            }

            let tid = maybe_pid_start..maybe_pid_end;

            // unwraps are safe because we've validated all preconditions
            (
                None,
                std::str::from_utf8(&src[pid]).unwrap().parse().unwrap(),
                std::str::from_utf8(&src[tid]).unwrap().parse().unwrap(),
            )
        };

        // expect a space
        if src[i] != b' ' {
            return self.enter_error_state(src);
        }
        i += 1;

        let level = match src[i] {
            b'V' => LogLevel::Verbose,
            b'D' => LogLevel::Debug,
            b'I' => LogLevel::Info,
            b'W' => LogLevel::Warning,
            b'E' => LogLevel::Error,
            b'F' => LogLevel::Fatal,
            x => LogLevel::Other(x),
        };
        i += 1;

        // expect a /
        if src[i] != b'/' {
            return self.enter_error_state(src);
        }
        i += 1;

        // everything to this point was covered by MINIMAL_LOG_LEN, but now we need to start checking len again

        // parse tag
        let tag_start = i;
        const HEADER_END: &[u8] = b" ]\n";

        while i < src.len() - HEADER_END.len() && &src[i..][..HEADER_END.len()] != HEADER_END {
            i += 1;
        }

        if i == src.len() - HEADER_END.len() {
            src.reserve(1024);
            return None;
        }

        let tag_end = i;

        // skip end of header
        i += HEADER_END.len();

        let message_start = i;
        // scan until we find the next message
        let packet_begins = [
            EXPECTED_BEGINNING_OF_BUFFER,
            EXPECTED_BEGINNING_OF_LOG_HEADER,
        ];
        let max_len = packet_begins.iter().map(|x| x.len()).max().unwrap();

        let message_end = loop {
            if i > src.len() - (2 + max_len) {
                src.reserve(1024);
                return None;
            }

            if src[i] == b'\n' && src[i + 1] == b'\n' {
                if src[i + 2..].starts_with(EXPECTED_BEGINNING_OF_BUFFER)
                    || src[i + 2..].starts_with(EXPECTED_BEGINNING_OF_LOG_HEADER)
                {
                    break i;
                }
            }
            i += 1;
        };

        let result = LogItem::LogMessage(LogLongMessage {
            timestamp,
            uid,
            pid,
            tid,
            level,
            tag: String::from_utf8_lossy(&src[tag_start..tag_end])
                .trim()
                .to_string(),
            message: src[message_start..message_end].to_owned(),
        });

        // skip past new lines
        src.advance(message_end + 2);

        Some(result)
    }
}

impl tokio_util::codec::Decoder for LogcatStringDecoder {
    type Item = LogItem;

    type Error = LogcatDecodeError;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        if self.is_in_error_state {
            return Ok(self.scan_out_error_state(src));
        }

        if src.is_empty() {
            return Ok(None);
        }

        let first_char = src[0];

        if first_char == b'-' {
            return Ok(self.decode_beginning_of_ring_buffer(src));
        }

        // neither beginning of ring buffer nor beginning of log? error case.
        if first_char != b'[' {
            return Ok(self.enter_error_state(src));
        }

        return Ok(self.decode_log(src));
    }
}

pub fn logcat(serial: &str) -> impl Stream<Item = Result<LogMessage, LogcatDecodeError>> {
    assert!(!serial.is_empty());

    let adb = super::get_adb()
        .arg("-s")
        .arg(serial)
        .args(shell_words::split("logcat -B").unwrap().as_slice())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();

    FramedRead::new(
        BufReader::new(adb.stdout.unwrap()),
        LogcatBinaryDecoder::new(),
    )
}

struct LogcatBinaryDecoder;

impl LogcatBinaryDecoder {
    fn new() -> Self {
        Self
    }
}

/// uint16_t len;   /* length of the payload */
/// uint16_t __pad; /* no matter what, we get 2 bytes of padding */
/// int32_t pid;    /* generating process's pid */
/// int32_t tid;    /* generating process's tid */
/// int32_t sec;    /* seconds since Epoch */
/// int32_t nsec;   /* nanoseconds */
const LOGGER_ENTRY_V1_SIZE: usize = 20;

/// uint16_t len;      /* length of the payload */
/// uint16_t hdr_size; /* sizeof(struct logger_entry_v3) */
/// int32_t pid;       /* generating process's pid */
/// int32_t tid;       /* generating process's tid */
/// int32_t sec;       /* seconds since Epoch */
/// int32_t nsec;      /* nanoseconds */
/// uint32_t lid;      /* log id of the payload */
#[allow(unused)]
const LOGGER_ENTRY_V3_SIZE: usize = 24;

/// uint16_t len;      /* length of the payload */
/// uint16_t hdr_size; /* sizeof(struct logger_entry) */
/// int32_t pid;       /* generating process's pid */
/// uint32_t tid;      /* generating process's tid */
/// uint32_t sec;      /* seconds since Epoch */
/// uint32_t nsec;     /* nanoseconds */
/// uint32_t lid;      /* log id of the payload, bottom 4 bits currently */
/// uint32_t uid;      /* generating process's uid */
const LOGGER_ENTRY_V4_SIZE: usize = 28;

const LOGGER_ENTRY_LEN_OFF: usize = 0;
const LOGGER_ENTRY_HDR_SIZE_OFF: usize = 2;
const LOGGER_ENTRY_PID_OFF: usize = 4;
const LOGGER_ENTRY_TID_OFF: usize = 8;
const LOGGER_ENTRY_SEC_OFF: usize = 12;
const LOGGER_ENTRY_NSEC_OFF: usize = 16;
const LOGGER_ENTRY_LID_OFF: usize = 20;
const LOGGER_ENTRY_UID_OFF: usize = 24;

// max entry size - in android 12, this is 5 * 1024, but pad out to 2^14 for better forward compat
const LOGGER_ENTRY_MAX_SIZE: usize = 1 << 14;

fn read_u32(src: &BytesMut, hdr_size: usize, off: usize) -> Option<u32> {
    if off > hdr_size - 4 {
        return None;
    }

    Some(u32::from_le_bytes([
        src[off],
        src[off + 1],
        src[off + 2],
        src[off + 3],
    ]))
}

impl tokio_util::codec::Decoder for LogcatBinaryDecoder {
    type Item = LogMessage;

    type Error = LogcatDecodeError;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        if src.is_empty() {
            return Ok(None);
        }

        if src.len() < LOGGER_ENTRY_PID_OFF {
            src.reserve(1024);
            return Ok(None);
        }

        let len: usize =
            u16::from_le_bytes([src[LOGGER_ENTRY_LEN_OFF], src[LOGGER_ENTRY_LEN_OFF + 1]]).into();

        // sanity check `len` is at least 8-bit level + two \0
        assert!(len >= 3, "len={len}");
        assert!(len <= LOGGER_ENTRY_MAX_SIZE, "len={len}");

        let hdr_size: usize = u16::from_le_bytes([
            src[LOGGER_ENTRY_HDR_SIZE_OFF],
            src[LOGGER_ENTRY_HDR_SIZE_OFF + 1],
        ])
        .into();

        // sanity check hdr_size
        assert!(
            hdr_size >= LOGGER_ENTRY_V1_SIZE,
            "header too small (hdr_size={hdr_size})"
        );

        assert_eq!(hdr_size % 4, 0, "hdr_size={hdr_size} not multiple of 4");

        // forward compatibility
        assert!(
            hdr_size <= LOGGER_ENTRY_V4_SIZE + 6 * std::mem::size_of::<u32>(),
            "Unreasonable header size={hdr_size}"
        );

        if src.len() < len + hdr_size {
            src.reserve(len + hdr_size - src.len() + LOGGER_ENTRY_PID_OFF);
            return Ok(None);
        }

        let pid = i32::from_le_bytes([
            src[LOGGER_ENTRY_PID_OFF],
            src[LOGGER_ENTRY_PID_OFF + 1],
            src[LOGGER_ENTRY_PID_OFF + 2],
            src[LOGGER_ENTRY_PID_OFF + 3],
        ]);

        let tid = read_u32(src, hdr_size, LOGGER_ENTRY_TID_OFF).unwrap();
        let sec = read_u32(src, hdr_size, LOGGER_ENTRY_SEC_OFF).unwrap();
        let nsec = read_u32(src, hdr_size, LOGGER_ENTRY_NSEC_OFF).unwrap();

        let lid = read_u32(src, hdr_size, LOGGER_ENTRY_LID_OFF);
        let uid = read_u32(src, hdr_size, LOGGER_ENTRY_UID_OFF);

        let is_binary = if let Some(lid) = lid {
            match lid {
                LOG_ID_EVENTS | LOG_ID_STATS | LOG_ID_SECURITY => true,
                _ => false,
            }
        } else {
            false
        };

        let buf = &src[hdr_size..][..len];

        let buffer = if is_binary {
            let tag = i32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]);
            LogBuffer::BinaryLog(BinaryLogBuffer { tag })
        } else {
            let level = match buf[0] {
                LOG_LEVEL_VERBOSE => LogLevel::Verbose,
                LOG_LEVEL_DEBUG => LogLevel::Debug,
                LOG_LEVEL_INFO => LogLevel::Info,
                LOG_LEVEL_WARN => LogLevel::Warning,
                LOG_LEVEL_ERROR => LogLevel::Error,
                LOG_LEVEL_FATAL => LogLevel::Fatal,
                x => LogLevel::Other(x),
            };

            // let tag = CStr::from_ptr(buf[1..].as_ptr() as *const c_char).unwrap();
            let tag_start = 1;
            let tag_end = buf[tag_start..]
                .iter()
                .copied()
                .enumerate()
                .map(|(i, c)| (i + tag_start, c))
                .find(|(_, x)| *x == 0)
                .map(|(i, _)| i)
                .unwrap();

            let tag = String::from_utf8_lossy(&buf[tag_start..tag_end]).into();

            let message_start = tag_end + 1;
            let message_end = buf[message_start..]
                .iter()
                .copied()
                .enumerate()
                .map(|(i, c)| (i + message_start, c))
                .find(|(_, x)| *x == 0)
                .map(|(i, _)| i)
                .unwrap_or(buf.len() - 1); // if the last character is not null, then `adb logcat` treats it as NULL

            let message = String::from_utf8_lossy(&buf[message_start..message_end])
                .trim_end_matches(|c: char| !c.is_ascii())
                .into();

            LogBuffer::TextLog(TextLogBuffer {
                level,
                tag,
                message,
            })
        };

        src.advance(hdr_size + len);

        Ok(Some(LogMessage {
            timestamp: NaiveDateTime::from_timestamp_opt(sec as i64, nsec).unwrap(),
            uid,
            pid,
            tid,
            lid,
            buffer,
        }))
    }
}
