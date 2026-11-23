use std::env;
use std::fs::File;
use std::io::{self, BufRead, BufReader, Write};
use std::path::Path;
use std::process::ExitCode;
use std::thread;
use std::time::{Duration, Instant};

const USAGE: &str = "Usage: telemetry-replay <file> [--speed <factor>]";
const HEADER: &str = "timestamp_ms,payload";

#[derive(Debug)]
struct Event {
    offset: Duration,
    payload: String,
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("telemetry-replay: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    let mut args = env::args_os().skip(1);
    let Some(path) = args.next() else {
        return Err(USAGE.to_owned());
    };
    let speed = match (args.next(), args.next(), args.next()) {
        (None, None, None) => 1.0,
        (Some(flag), Some(value), None) if flag == "--speed" => value
            .to_str()
            .and_then(|value| value.parse::<f64>().ok())
            .filter(|speed| speed.is_finite() && *speed > 0.0)
            .ok_or_else(|| "speed must be a positive finite number".to_owned())?,
        _ => return Err(USAGE.to_owned()),
    };

    let path = Path::new(&path);
    let file = File::open(path).map_err(|error| format!("cannot open {path:?}: {error}"))?;
    let events =
        load_schedule(BufReader::new(file), speed).map_err(|error| format!("{path:?}: {error}"))?;
    replay(events, &mut io::stdout().lock())
        .map_err(|error| format!("cannot write output: {error}"))
}

fn load_schedule(reader: impl BufRead, speed: f64) -> Result<Vec<Event>, String> {
    // ponytail: preloading validates everything before playback and uses memory proportional
    // to the capture. For large captures, validate once and stream them on a second pass.
    let mut events = Vec::new();
    let mut has_header = false;
    let mut first_timestamp = None;
    let mut previous_timestamp = None;

    for (index, line) in reader.lines().enumerate() {
        let number = index + 1;
        let line = line.map_err(|error| format!("line {number}: cannot read input: {error}"))?;
        if line.trim().is_empty() {
            continue;
        }
        if !has_header {
            if line.trim() != HEADER {
                return Err(format!("line {number}: expected header {HEADER}"));
            }
            has_header = true;
            continue;
        }

        let (timestamp, payload) = line
            .split_once(',')
            .ok_or_else(|| format!("line {number}: expected timestamp_ms,payload"))?;
        let timestamp = timestamp.trim().parse::<u64>().map_err(|_| {
            format!("line {number}: timestamp_ms must be an unsigned 64-bit integer")
        })?;
        if payload.trim().is_empty() {
            return Err(format!("line {number}: payload is empty"));
        }
        if previous_timestamp.is_some_and(|previous| timestamp < previous) {
            return Err(format!("line {number}: timestamp_ms goes backwards"));
        }

        let first = *first_timestamp.get_or_insert(timestamp);
        let recorded = Duration::from_millis(timestamp - first);
        let offset = Duration::try_from_secs_f64(recorded.as_secs_f64() / speed)
            .map_err(|_| format!("line {number}: playback offset is out of range"))?;
        events.push(Event {
            offset,
            payload: payload.to_owned(),
        });
        previous_timestamp = Some(timestamp);
    }

    if events.is_empty() {
        return Err("no telemetry events".to_owned());
    }
    Ok(events)
}

fn replay(events: Vec<Event>, output: &mut impl Write) -> io::Result<()> {
    let start = Instant::now();
    for event in events {
        let wait = event.offset.saturating_sub(start.elapsed());
        if !wait.is_zero() {
            thread::sleep(wait);
        }
        writeln!(output, "{}", event.payload)?;
        output.flush()?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{load_schedule, Duration};

    #[test]
    fn schedules_the_sample_at_three_speeds() {
        let input = include_bytes!("../sample.csv");
        for (speed, expected_ms) in [
            (1.0, [0, 500, 1500]),
            (2.0, [0, 250, 750]),
            (0.5, [0, 1000, 3000]),
        ] {
            let events = load_schedule(&input[..], speed).unwrap();
            let offsets: Vec<_> = events.iter().map(|event| event.offset).collect();
            assert_eq!(offsets, expected_ms.map(Duration::from_millis));
        }
    }

    #[test]
    fn preserves_input_order_at_equal_timestamps_and_keeps_payloads() {
        let input = b"timestamp_ms,payload\r\n9,imu,0.12\r\n\r\n9, second \r\n";
        let events = load_schedule(&input[..], 1.0).unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].payload, "imu,0.12");
        assert_eq!(events[1].payload, " second ");
        assert!(events.iter().all(|event| event.offset.is_zero()));
    }

    #[test]
    fn keeps_millisecond_gaps_at_large_absolute_timestamps() {
        let input = b"timestamp_ms,payload\n18446744073709551614,a\n18446744073709551615,b\n";
        let events = load_schedule(&input[..], 2.0).unwrap();
        assert_eq!(events[1].offset, Duration::from_micros(500));
    }

    #[test]
    fn reports_the_line_of_a_backwards_timestamp() {
        let input = b"timestamp_ms,payload\n100,a\n\n300,b\n200,c\n";
        let error = load_schedule(&input[..], 1.0).unwrap_err();
        assert_eq!(error, "line 5: timestamp_ms goes backwards");
    }

    #[test]
    fn rejects_bad_input_before_playback() {
        for (input, expected) in [
            ("", "no telemetry events"),
            ("timestamp_ms,payload\n", "no telemetry events"),
            (
                "other,columns\n",
                "line 1: expected header timestamp_ms,payload",
            ),
            (
                "timestamp_ms,payload\n0\n",
                "line 2: expected timestamp_ms,payload",
            ),
            ("timestamp_ms,payload\n0, \n", "line 2: payload is empty"),
            (
                "timestamp_ms,payload\n-1,a\n",
                "line 2: timestamp_ms must be an unsigned 64-bit integer",
            ),
        ] {
            assert_eq!(load_schedule(input.as_bytes(), 1.0).unwrap_err(), expected);
        }
    }

    #[test]
    fn an_unrepresentable_offset_returns_an_error() {
        let input = b"timestamp_ms,payload\n0,a\n1,b\n";
        let error = load_schedule(&input[..], 1e-30).unwrap_err();
        assert_eq!(error, "line 3: playback offset is out of range");
    }
}
