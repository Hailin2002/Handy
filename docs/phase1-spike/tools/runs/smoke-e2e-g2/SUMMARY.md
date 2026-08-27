# Meeting Loopback Spike - CSV Analysis

Log: C:\Users\haeni\Documents\Projekte\Speak system\docs\phase1-spike\tools\runs\smoke-e2e-g2\meeting_loopback_spike_smoke.csv

## Run metadata

- meeting_loopback_spike: run
- microphone_device: Internal AUX Jack (Realtek USB2.0 Audio)
- loopback_device: Speakers (Focusrite USB Audio)
- duration_s: 90
- silence_threshold: 0.0001
- microphone_config: 48000:2:f32
- loopback_config: 48000:2:f32
- streams_started:
- run_finished: elapsed_s=90.120
- SUMMARY: source=loopback device="Speakers (Focusrite USB Audio)" callbacks=3031 total_frames=1454112 wall_s=38.142 drift_frames=-376722.7 max_gap_ms=7832.556 gaps(normal/sparse/long)=3030/0/1 silent_frac=0.0129 stream_errors=0 config=48000:2:f32

## Source: microphone

| Metric                                                    | Value                                             |
| --------------------------------------------------------- | ------------------------------------------------- |
| Callbacks                                                 | 8990                                              |
| Stream errors                                             | 0                                                 |
| Config                                                    | 48000 Hz, 2 ch, f32                               |
| Measurement span (last callback)                          | 90.1 s                                            |
| Total frames                                              | 4313280                                           |
| Max gap                                                   | 91.764 ms                                         |
| Gap histogram (normal <=250 ms / sparse <=1 s / gap >1 s) | 8990 / 0 / 0                                      |
| Time in sparse gaps                                       | 0 s                                               |
| Time in long gaps (>1 s)                                  | 0 s                                               |
| Drift at end                                              | -12554.2 frames (-261.546 ms)                     |
| Drift range                                               | -13464.6 .. -3921.9 frames                        |
| Drift trend (stable window >10 s)                         | -6706.7 to -12554.2 frames; rate -262744 frames/h |
| Silent fraction (callbacks)                               | 98.95 % (8896/8990)                               |

## Source: loopback

| Metric                                                    | Value                                                  |
| --------------------------------------------------------- | ------------------------------------------------------ |
| Callbacks                                                 | 3031                                                   |
| Stream errors                                             | 0                                                      |
| Config                                                    | 48000 Hz, 2 ch, f32                                    |
| Measurement span (last callback)                          | 38.1 s                                                 |
| Total frames                                              | 1454112                                                |
| Max gap                                                   | 7832.556 ms                                            |
| Gap histogram (normal <=250 ms / sparse <=1 s / gap >1 s) | 3030 / 0 / 1                                           |
| Time in sparse gaps                                       | 0 s                                                    |
| Time in long gaps (>1 s)                                  | 7.833 s                                                |
| Drift at end                                              | -376722.7 frames (-7848.39 ms)                         |
| Drift range                                               | -376991.1 .. -375474.9 frames                          |
| Drift trend (stable window >10 s)                         | -375477.3 to -376722.7 frames; rate -159326.5 frames/h |
| Silent fraction (callbacks)                               | 1.29 % (39/3031)                                       |

### Top 10 largest gaps

| t_rel_ms | gap_ms   | class  | silent |
| -------- | -------- | ------ | ------ |
| 7832.6   | 7832.556 | gap    | false  |
| 10817.1  | 34.508   | normal | false  |
| 13386    | 32.576   | normal | false  |
| 25992.9  | 20.296   | normal | false  |
| 34533.1  | 19.94    | normal | false  |
| 17052.6  | 18.792   | normal | false  |
| 10450.9  | 18.13    | normal | false  |
| 34374.9  | 12.146   | normal | false  |
| 15934.5  | 12.016   | normal | false  |
| 8334     | 11.31    | normal | false  |

## Cross-source comparison

- Relative drift loopback vs microphone at end: -364168.5 frames (-7586.844 ms) after 38.1 s
- Relative drift rate: -34371381.4 frames/h
- Max gap microphone: 91.764 ms; loopback: 7832.556 ms

## Event log (scenario markers)

```text
2026-08-24 08:52:52.509+02:00 RUN_LAUNCH exe=C:\Users\haeni\Documents\Projekte\Speak system\src-tauri\target\debug\meeting_loopback_spike.exe duration_s=90
2026-08-24 08:52:52.531+02:00 RUN_STARTED pid=35756
2026-08-24 08:52:57.620+02:00 TONE_START t_rel_s=5.1
2026-08-24 08:53:30.415+02:00 TONE_END t_rel_s=37.9
2026-08-24 08:54:22.702+02:00 RUN_FINISHED exitcode=
2026-08-24 08:54:22.704+02:00 ANALYZE_START
```
