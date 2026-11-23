# telemetry-replay

`telemetry-replay` is a Rust CLI for replaying recorded sensor data at an
adjustable speed. It uses the standard library and writes each payload to
stdout as it becomes due, ready for a pipe into a downstream program.

## Run the demo

```sh
cargo build
time ./target/debug/telemetry-replay sample.csv
time ./target/debug/telemetry-replay sample.csv --speed 2
```

Both runs emit the same payloads in the same order.

```text
imu_ax=0.12
imu_ax=0.14
imu_ax=0.10
```

| Timestamp in the capture | Target at 1× | Target at 2× |
| ---: | ---: | ---: |
| 10000 ms | 0 ms | 0 ms |
| 10500 ms | 500 ms | 250 ms |
| 11500 ms | 1500 ms | 750 ms |

## Input

Use a UTF-8 file whose first nonblank line is `timestamp_ms,payload`. Each
event starts with a nonnegative 64-bit integer in milliseconds, followed by
a comma and the payload. Everything after the first comma belongs to the
payload. Blank lines are skipped. Timestamps must be nondecreasing. Ties
are replayed in file order.

`--speed` defaults to 1 and accepts any positive, finite factor, including
`0.5` for half speed. Invalid records and unrepresentable offsets report
the file and line number on stderr and exit with an error before playback.

## Scheduling

The first timestamp becomes time zero. Each target offset is computed as
`(timestamp - first_timestamp) / speed`. The replay loop compares each
target with time elapsed from one monotonic `Instant`. This prevents a
late wake-up from being added to every later target. When a target is
already past, its payload is emitted immediately. Actual delivery depends
on the OS scheduler and the stdout consumer.

The CLI validates and loads the schedule before starting the clock, so its
memory use grows with the capture size.

## Run the tests

```sh
cargo test
```

The tests validate target offsets and input errors without sleeping.
