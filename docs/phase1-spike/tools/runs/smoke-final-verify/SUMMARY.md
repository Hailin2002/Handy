# Meeting Loopback Spike - CSV Analysis

Log: C:\Users\haeni\Documents\Projekte\Speak system\docs\phase1-spike\tools\runs\smoke-final-verify\meeting_loopback_spike_smoke.csv

## Run metadata

- meeting_loopback_spike: run
- microphone_device: Internal AUX Jack (Realtek USB2.0 Audio)
- loopback_device: Speakers (Focusrite USB Audio)
- duration_s: 90
- silence_threshold: 0.0001
- microphone_config: 48000:2:f32
- loopback_config: 48000:2:f32
- streams_started:
- run_finished: elapsed_s=90.057
- SUMMARY: source=loopback device="Speakers (Focusrite USB Audio)" callbacks=3034 total_frames=1455936 wall_s=36.068 drift_frames=-275332.2 max_gap_ms=5708.989 gaps(normal/sparse/long)=3033/0/1 silent_frac=0.0129 stream_errors=0 config=48000:2:f32

## Source: microphone

| Metric                                                    | Value                                               |
| --------------------------------------------------------- | --------------------------------------------------- |
| Callbacks                                                 | 8990                                                |
| Stream errors                                             | 0                                                   |
| Config                                                    | 48000 Hz, 2 ch, f32                                 |
| Measurement span (last callback)                          | 90.1 s                                              |
| Total frames                                              | 4310592                                             |
| Max gap                                                   | 78.538 ms                                           |
| Gap histogram (normal <=250 ms / sparse <=1 s / gap >1 s) | 8990 / 0 / 0                                        |
| Time in sparse gaps                                       | 0 s                                                 |
| Time in long gaps (>1 s)                                  | 0 s                                                 |
| Drift at end                                              | -13167.1 frames (-274.315 ms)                       |
| Drift range                                               | -13547.9 .. -3282.2 frames                          |
| Drift trend (stable window >10 s)                         | -3284.9 to -13167.1 frames; rate -444310.9 frames/h |
| Silent fraction (callbacks)                               | 98.94 % (8895/8990)                                 |

## Source: loopback

| Metric                                                    | Value                                                  |
| --------------------------------------------------------- | ------------------------------------------------------ |
| Callbacks                                                 | 3034                                                   |
| Stream errors                                             | 0                                                      |
| Config                                                    | 48000 Hz, 2 ch, f32                                    |
| Measurement span (last callback)                          | 36.1 s                                                 |
| Total frames                                              | 1455936                                                |
| Max gap                                                   | 5708.989 ms                                            |
| Gap histogram (normal <=250 ms / sparse <=1 s / gap >1 s) | 3033 / 0 / 1                                           |
| Time in sparse gaps                                       | 0 s                                                    |
| Time in long gaps (>1 s)                                  | 5.709 s                                                |
| Drift at end                                              | -275332.2 frames (-5736.088 ms)                        |
| Drift range                                               | -276261.7 .. -273501.4 frames                          |
| Drift trend (stable window >10 s)                         | -273519.6 to -275332.2 frames; rate -250399.8 frames/h |
| Silent fraction (callbacks)                               | 1.29 % (39/3034)                                       |

### Top 10 largest gaps

| t_rel_ms | gap_ms   | class  | silent |
| -------- | -------- | ------ | ------ |
| 5709     | 5708.989 | gap    | false  |
| 24350.9  | 62.401   | normal | false  |
| 32437.5  | 29.499   | normal | false  |
| 29653.9  | 25.689   | normal | false  |
| 31109.4  | 21.376   | normal | false  |
| 34359.6  | 21.149   | normal | false  |
| 35389.5  | 21.077   | normal | false  |
| 34779.4  | 21.044   | normal | false  |
| 32469.3  | 20.641   | normal | false  |
| 32208.7  | 20.595   | normal | false  |

## Cross-source comparison

- Relative drift loopback vs microphone at end: -262165.1 frames (-5461.773 ms) after 36.1 s
- Relative drift rate: -26167019.4 frames/h
- Max gap microphone: 78.538 ms; loopback: 5708.989 ms

## Event log (scenario markers)

```text
2026-08-24 09:00:35.937+02:00 RUN_LAUNCH exe=C:\Users\haeni\Documents\Projekte\Speak system\src-tauri\target\debug\meeting_loopback_spike.exe duration_s=90
2026-08-24 09:00:35.975+02:00 RUN_STARTED pid=52288
2026-08-24 09:00:40.992+02:00 TONE_START t_rel_s=5.1
2026-08-24 09:01:11.940+02:00 TONE_END t_rel_s=36
2026-08-24 09:02:06.157+02:00 RUN_FINISHED exitcode=0
2026-08-24 09:02:06.162+02:00 ANALYZE_START
```
