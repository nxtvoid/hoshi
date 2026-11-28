//! Workbooks are read in a process of their own: the app's program, started to read one sheet
//! and write it to its output. A crafted workbook can have its reader set aside far more memory than the
//! file holds, or never finish; then only that process ends, held to [`MEMORY`] and
//! [`PATIENCE`].

use super::{MAX_CELLS, TableFailure, workbook};
use calamine::{CellErrorType, Data, ExcelDateTime, ExcelDateTimeType, Range};
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::Path;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

/// The argument that starts the program as a workbook's reader.
const FLAG: &str = "--read-workbook";
const PATH: &str = "HOSHI_WORKBOOK";
const SHEET: &str = "HOSHI_WORKBOOK_SHEET";
/// What the reader writes before the sheet, past anything else the program writes.
const MARK: &[u8] = b"\0hoshi-workbook\0";
/// How much output may come before [`MARK`].
const MAX_BEFORE_MARK: usize = 64 * 1024;
/// The most memory the reading process holds.
const MEMORY: usize = 3 << 30;
/// The memory past which the reading process stops on its own, before an allocation is refused
/// and it aborts.
const SOFT_MEMORY: usize = MEMORY / 8 * 7;
/// How long a workbook is given to be read.
#[cfg(not(test))]
const PATIENCE: Duration = Duration::from_secs(120);
#[cfg(test)]
const PATIENCE: Duration = Duration::from_secs(5);
/// The reader's exit code when it couldn't write what it read.
const UNWRITTEN: i32 = 2;
/// The reader's exit code when it panicked.
const PANICKED: i32 = 3;
/// The reader's exit code when it passed [`SOFT_MEMORY`] or [`PATIENCE`].
const EXHAUSTED: i32 = 4;
/// The reader's exit code when the program that started it is gone.
const ABANDONED: i32 = 5;
/// The largest date or duration, in days, sent as one: past it, it is sent as a number.
const MAX_DAYS: f64 = 10_000_000.;

/// A sheet as read: the names of the workbook's visible worksheets, and the cells of the sheet
/// asked for or else the first.
pub type Opened = (Vec<String>, Range<Data>);

/// What the test program allocates, held to [`MEMORY`] when it reads a workbook.
#[cfg(test)]
#[global_allocator]
static ALLOCATOR: cap::Cap<std::alloc::System> = cap::Cap::new(std::alloc::System, usize::MAX);

fn failed(error: std::io::Error) -> TableFailure {
    TableFailure::Read(error.to_string())
}

/// Reads sheet `sheet` of the workbook at `path`, or its first visible one, in a process of its
/// own. One that runs out of memory or time is too large.
pub fn read(path: &Path, sheet: Option<&str>) -> Result<Opened, TableFailure> {
    let mut command = command().map_err(failed)?;
    command
        .env(PATH, path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    match sheet {
        Some(sheet) => command.env(SHEET, sheet),
        None => command.env_remove(SHEET),
    };
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    let mut child = command.spawn().map_err(failed)?;
    let held_open = child.stdin.take();
    let output = child.stdout.take().ok_or_else(|| failed(malformed()))?;
    let reading = std::thread::spawn(move || {
        let mut output = BufReader::new(output);
        past_mark(&mut output).map_err(failed)?;
        decode(&mut output)
    });
    let waited = wait(&mut child);
    drop(held_open);
    let read = reading.join().unwrap_or_else(|_| Err(failed(malformed())));
    let Some(status) = waited.map_err(failed)? else {
        return Err(TableFailure::TooLarge);
    };
    match status.code() {
        Some(0) => read,
        Some(UNWRITTEN | PANICKED) => Err(TableFailure::Read(format!(
            "the workbook's reader stopped: {status}"
        ))),
        _ => Err(TableFailure::TooLarge),
    }
}

/// The running program, which on Linux is still there when an update replaced its file.
#[cfg(not(test))]
fn command() -> std::io::Result<Command> {
    #[cfg(target_os = "linux")]
    let program = std::path::PathBuf::from("/proc/self/exe");
    #[cfg(not(target_os = "linux"))]
    let program = std::env::current_exe()?;
    let mut command = Command::new(program);
    command.arg(FLAG);
    Ok(command)
}

/// The test program, running only the test that reads a workbook.
#[cfg(test)]
fn command() -> std::io::Result<Command> {
    let mut command = Command::new(std::env::current_exe()?);
    command.args([
        "table::reader::tests::reads_a_workbook_when_asked",
        "--exact",
        "--test-threads=1",
    ]);
    Ok(command)
}

/// How `child` exited; `None` when it ran out of time, and was ended.
fn wait(child: &mut Child) -> std::io::Result<Option<ExitStatus>> {
    let deadline = Instant::now() + PATIENCE;
    let mut pause = Duration::from_millis(2);
    loop {
        let status = child.try_wait().inspect_err(|_| {
            let _ = child.kill();
            let _ = child.wait();
        })?;
        if let Some(status) = status {
            return Ok(Some(status));
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Ok(None);
        }
        std::thread::sleep(pause);
        pause = (pause * 2).min(Duration::from_millis(50));
    }
}

/// Reads the workbook the program was started for, when it was started as its reader, and
/// gives the code to exit with: `limit` holds what the program allocates to the bytes given,
/// and `allocated` tells how much it holds.
pub fn serve(limit: fn(usize), allocated: fn() -> usize) -> Option<i32> {
    (std::env::args_os().nth(1)? == FLAG).then(|| serve_request(limit, allocated))
}

fn serve_request(limit: fn(usize), allocated: fn() -> usize) -> i32 {
    std::panic::set_hook(Box::new(|_| std::process::exit(PANICKED)));
    limit(MEMORY);
    std::thread::spawn(move || {
        let started = Instant::now();
        while started.elapsed() < PATIENCE {
            if allocated() > SOFT_MEMORY {
                std::process::exit(EXHAUSTED);
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        std::process::exit(EXHAUSTED);
    });
    std::thread::spawn(|| {
        let _ = std::io::stdin().read(&mut [0]);
        std::process::exit(ABANDONED);
    });
    let Some(path) = std::env::var_os(PATH) else {
        return UNWRITTEN;
    };
    let sheet = std::env::var(SHEET).ok();
    let read = read_here(Path::new(&path), sheet.as_deref());
    let mut out = BufWriter::new(std::io::stdout().lock());
    let written = out
        .write_all(MARK)
        .and_then(|()| encode(&mut out, &read))
        .and_then(|()| out.flush());
    if written.is_ok() { 0 } else { UNWRITTEN }
}

/// Reads `input` up to past [`MARK`].
fn past_mark(input: &mut impl Read) -> std::io::Result<()> {
    let mut matched = 0;
    for _ in 0..MAX_BEFORE_MARK + MARK.len() {
        let next = byte(input)?;
        matched = if next == MARK[matched] {
            matched + 1
        } else {
            usize::from(next == MARK[0])
        };
        if matched == MARK.len() {
            return Ok(());
        }
    }
    Err(malformed())
}

fn read_here(path: &Path, sheet: Option<&str>) -> Result<Opened, TableFailure> {
    let mut book = workbook::open(path)?;
    let sheets = workbook::sheets(&book);
    let range = match sheet.or(sheets.first().map(String::as_str)) {
        Some(name) => workbook::sheet(&mut book, name)?,
        None => Range::default(),
    };
    Ok((sheets, range))
}

const ERRORS: [CellErrorType; 8] = [
    CellErrorType::Div0,
    CellErrorType::NA,
    CellErrorType::Name,
    CellErrorType::Null,
    CellErrorType::Num,
    CellErrorType::Ref,
    CellErrorType::Value,
    CellErrorType::GettingData,
];

fn write_u32(out: &mut impl Write, number: u32) -> std::io::Result<()> {
    out.write_all(&number.to_le_bytes())
}

fn write_text(out: &mut impl Write, text: &str) -> std::io::Result<()> {
    write_u32(out, text.len() as u32)?;
    out.write_all(text.as_bytes())
}

fn encode(out: &mut impl Write, read: &Result<Opened, TableFailure>) -> std::io::Result<()> {
    let (sheets, range) = match read {
        Err(TableFailure::TooLarge) => return out.write_all(&[1]),
        Err(TableFailure::Read(reason)) => {
            out.write_all(&[2])?;
            return write_text(out, reason);
        }
        Ok(read) => read,
    };
    out.write_all(&[0])?;
    write_u32(out, sheets.len() as u32)?;
    for sheet in sheets {
        write_text(out, sheet)?;
    }
    let (Some(start), Some(end)) = (range.start(), range.end()) else {
        return out.write_all(&[0]);
    };
    out.write_all(&[1])?;
    for number in [start.0, start.1, end.0, end.1] {
        write_u32(out, number)?;
    }
    for data in range.rows().flatten() {
        encode_data(out, data)?;
    }
    Ok(())
}

/// A date as a serial day of the 1900 system, which a workbook may have counted from 1904.
fn serial_1900(date: &ExcelDateTime) -> f64 {
    let as_1900 = ExcelDateTime::new(date.as_f64(), ExcelDateTimeType::DateTime, false);
    if as_1900.as_datetime() == date.as_datetime() {
        date.as_f64()
    } else {
        date.as_f64() + 1462.
    }
}

fn encode_data(out: &mut impl Write, data: &Data) -> std::io::Result<()> {
    match data {
        Data::Empty => out.write_all(&[0]),
        Data::Int(number) => {
            out.write_all(&[1])?;
            out.write_all(&number.to_le_bytes())
        }
        Data::Float(number) => {
            out.write_all(&[2])?;
            out.write_all(&number.to_le_bytes())
        }
        Data::String(text) => {
            out.write_all(&[3])?;
            write_text(out, text)
        }
        Data::Bool(value) => out.write_all(&[4, u8::from(*value)]),
        Data::DateTime(date) if !date.as_f64().is_finite() || date.as_f64().abs() > MAX_DAYS => {
            out.write_all(&[2])?;
            out.write_all(&date.as_f64().to_le_bytes())
        }
        Data::DateTime(date) if date.is_duration() => {
            out.write_all(&[6])?;
            out.write_all(&date.as_f64().to_le_bytes())
        }
        Data::DateTime(date) => {
            out.write_all(&[5])?;
            out.write_all(&serial_1900(date).to_le_bytes())
        }
        Data::DateTimeIso(text) => {
            out.write_all(&[7])?;
            write_text(out, text)
        }
        Data::DurationIso(text) => {
            out.write_all(&[8])?;
            write_text(out, text)
        }
        Data::Error(error) => {
            let index = ERRORS.iter().position(|known| known == error).unwrap_or(0);
            out.write_all(&[9, index as u8])
        }
    }
}

fn byte(input: &mut impl Read) -> std::io::Result<u8> {
    let mut byte = [0];
    input.read_exact(&mut byte)?;
    Ok(byte[0])
}

fn read_u32(input: &mut impl Read) -> std::io::Result<u32> {
    let mut bytes = [0; 4];
    input.read_exact(&mut bytes)?;
    Ok(u32::from_le_bytes(bytes))
}

fn read_8(input: &mut impl Read) -> std::io::Result<[u8; 8]> {
    let mut bytes = [0; 8];
    input.read_exact(&mut bytes)?;
    Ok(bytes)
}

fn read_text(input: &mut impl Read) -> std::io::Result<String> {
    let mut bytes = vec![0; read_u32(input)? as usize];
    input.read_exact(&mut bytes)?;
    String::from_utf8(bytes).map_err(std::io::Error::other)
}

fn malformed() -> std::io::Error {
    std::io::Error::other("the workbook's reader sent something unreadable")
}

fn decode(input: &mut impl Read) -> Result<Opened, TableFailure> {
    match byte(input).map_err(failed)? {
        0 => decode_read(input).map_err(failed),
        1 => Err(TableFailure::TooLarge),
        2 => Err(TableFailure::Read(read_text(input).map_err(failed)?)),
        _ => Err(failed(malformed())),
    }
}

fn decode_read(input: &mut impl Read) -> std::io::Result<Opened> {
    let sheets = (0..read_u32(input)?)
        .map(|_| read_text(input))
        .collect::<Result<Vec<_>, _>>()?;
    if byte(input)? == 0 {
        return Ok((sheets, Range::default()));
    }
    let [top, left, bottom, right] = [(); 4].map(|_| read_u32(input));
    let (start, end) = ((top?, left?), (bottom?, right?));
    if end.0 < start.0 || end.1 < start.1 {
        return Err(malformed());
    }
    let cells = (u64::from(end.0 - start.0) + 1) * (u64::from(end.1 - start.1) + 1);
    if cells > MAX_CELLS {
        return Err(malformed());
    }
    let mut range = Range::new(start, end);
    for row in start.0..=end.0 {
        for column in start.1..=end.1 {
            let data = decode_data(input)?;
            if data != Data::Empty {
                range.set_value((row, column), data);
            }
        }
    }
    Ok((sheets, range))
}

fn decode_data(input: &mut impl Read) -> std::io::Result<Data> {
    Ok(match byte(input)? {
        0 => Data::Empty,
        1 => Data::Int(i64::from_le_bytes(read_8(input)?)),
        2 => Data::Float(f64::from_le_bytes(read_8(input)?)),
        3 => Data::String(read_text(input)?),
        4 => Data::Bool(byte(input)? != 0),
        5 => Data::DateTime(ExcelDateTime::new(
            f64::from_le_bytes(read_8(input)?),
            ExcelDateTimeType::DateTime,
            false,
        )),
        6 => Data::DateTime(ExcelDateTime::new(
            f64::from_le_bytes(read_8(input)?),
            ExcelDateTimeType::TimeDelta,
            false,
        )),
        7 => Data::DateTimeIso(read_text(input)?),
        8 => Data::DurationIso(read_text(input)?),
        9 => Data::Error(
            ERRORS
                .get(byte(input)? as usize)
                .cloned()
                .ok_or_else(malformed)?,
        ),
        _ => return Err(malformed()),
    })
}

#[cfg(test)]
mod tests;
