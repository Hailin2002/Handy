# Meeting Loopback Spike - CSV Analysis

Log: C:\Users\haeni\Documents\Projekte\Speak system\docs\phase1-spike\tools\runs\smoke-e2e-g1\meeting_loopback_spike_smoke.csv

## Run metadata

- meeting_loopback_spike: run
- microphone_device: Internal AUX Jack (Realtek USB2.0 Audio)
- loopback_device: Speakers (Focusrite USB Audio)
- duration_s: 90
- silence_threshold: 0.0001
- microphone_config: 48000:2:f32
- loopback_config: 48000:2:f32
- streams_started:
- run_finished: elapsed_s=90.183
- SUMMARY: source=loopback device="Speakers (Focusrite USB Audio)" callbacks=3029 total_frames=1453152 wall_s=36.187 drift_frames=-283803.3 max_gap_ms=5835.480 gaps(normal/sparse/long)=3028/0/1 silent_frac=0.0129 stream_errors=0 config=48000:2:f32

## Source: microphone

| Metric                                                    | Value                                             |
| --------------------------------------------------------- | ------------------------------------------------- |
| Callbacks                                                 | 8988                                              |
| Stream errors                                             | 0                                                 |
| Config                                                    | 48000 Hz, 2 ch, f32                               |
| Measurement span (last callback)                          | 90.2 s                                            |
| Total frames                                              | 4311936                                           |
| Max gap                                                   | 97.388 ms                                         |
| Gap histogram (normal <=250 ms / sparse <=1 s / gap >1 s) | 8988 / 0 / 0                                      |
| Time in sparse gaps                                       | 0 s                                               |
| Time in long gaps (>1 s)                                  | 0 s                                               |
| Drift at end                                              | -17033.3 frames (-354.86 ms)                      |
| Drift range                                               | -17382 .. -4172.2 frames                          |
| Drift trend (stable window >10 s)                         | -4175 to -17033.3 frames; rate -577325.4 frames/h |
| Silent fraction (callbacks)                               | 99.81 % (8971/8988)                               |

## Source: loopback

| Metric                                                    | Value                                                  |
| --------------------------------------------------------- | ------------------------------------------------------ |
| Callbacks                                                 | 3029                                                   |
| Stream errors                                             | 0                                                      |
| Config                                                    | 48000 Hz, 2 ch, f32                                    |
| Measurement span (last callback)                          | 36.2 s                                                 |
| Total frames                                              | 1453152                                                |
| Max gap                                                   | 5835.48 ms                                             |
| Gap histogram (normal <=250 ms / sparse <=1 s / gap >1 s) | 3028 / 0 / 1                                           |
| Time in sparse gaps                                       | 0 s                                                    |
| Time in long gaps (>1 s)                                  | 5.835 s                                                |
| Drift at end                                              | -283803.3 frames (-5912.569 ms)                        |
| Drift range                                               | -284028 .. -279132.1 frames                            |
| Drift trend (stable window >10 s)                         | -279160.1 to -283803.3 frames; rate -638466.5 frames/h |
| Silent fraction (callbacks)                               | 1.29 % (39/3029)                                       |

### Top 10 largest gaps

| t_rel_ms | gap_ms  | class  | silent |
| -------- | ------- | ------ | ------ |
| 5835.5   | 5835.48 | gap    | false  |
| 30839.2  | 63.136  | normal | false  |
| 24467.9  | 62.584  | normal | false  |
| 21376.8  | 11.493  | normal | false  |
| 32466.7  | 11.436  | normal | false  |
| 17457.1  | 11.424  | normal | false  |
| 26036.7  | 11.402  | normal | false  |
| 10316.7  | 11.363  | normal | false  |
| 15036.6  | 11.315  | normal | false  |
| 23226.8  | 11.313  | normal | false  |

## Cross-source comparison

- Relative drift loopback vs microphone at end: -266770 frames (-5557.708 ms) after 36.2 s
- Relative drift rate: -26539460.7 frames/h
- Max gap microphone: 97.388 ms; loopback: 5835.48 ms

## Event log (scenario markers)

```text
2026-08-24 08:50:13.374+02:00 RUN_LAUNCH exe=C:\Users\haeni\Documents\Projekte\Speak system\src-tauri\target\debug\meeting_loopback_spike.exe duration_s=90
2026-08-24 08:50:15.392+02:00 RUN_STARTED pid=2888
2026-08-24 08:50:20.414+02:00 TONE_START t_rel_s=7
2026-08-24 08:50:51.544+02:00 TONE_END t_rel_s=38.2
2026-08-24 08:51:45.661+02:00 RUN_FINISHED exitcode=
2026-08-24 08:51:45.664+02:00 ANALYZE_START
```
