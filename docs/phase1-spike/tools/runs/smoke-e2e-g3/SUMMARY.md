# Meeting Loopback Spike - CSV Analysis

Log: C:\Users\haeni\Documents\Projekte\Speak system\docs\phase1-spike\tools\runs\smoke-e2e-g3\meeting_loopback_spike_smoke.csv

## Run metadata

- meeting_loopback_spike: run
- microphone_device: Internal AUX Jack (Realtek USB2.0 Audio)
- loopback_device: Speakers (Focusrite USB Audio)
- duration_s: 90
- silence_threshold: 0.0001
- microphone_config: 48000:2:f32
- loopback_config: 48000:2:f32
- streams_started: 
- run_finished: elapsed_s=90.062
- SUMMARY: source=loopback device="Speakers (Focusrite USB Audio)" callbacks=3030 total_frames=1453248 wall_s=35.788 drift_frames=-264589.0 max_gap_ms=5407.722 gaps(normal/sparse/long)=3029/0/1 silent_frac=0.0129 stream_errors=0 config=48000:2:f32

## Source: microphone

| Metric | Value |
| --- | --- |
| Callbacks | 8985 |
| Stream errors | 0 |
| Config | 48000 Hz, 2 ch, f32 |
| Measurement span (last callback) | 90.1 s |
| Total frames | 4310496 |
| Max gap | 82.495 ms |
| Gap histogram (normal <=250 ms / sparse <=1 s / gap >1 s) | 8985 / 0 / 0 |
| Time in sparse gaps | 0 s |
| Time in long gaps (>1 s) | 0 s |
| Drift at end | -14075.7 frames (-293.244 ms) |
| Drift range | -14279.3 .. -2655.9 frames |
| Drift trend (stable window >10 s) | -2657.2 to -14075.7 frames; rate -513255.8 frames/h |
| Silent fraction (callbacks) | 98.95 % (8891/8985) |

## Source: loopback

| Metric | Value |
| --- | --- |
| Callbacks | 3030 |
| Stream errors | 0 |
| Config | 48000 Hz, 2 ch, f32 |
| Measurement span (last callback) | 35.8 s |
| Total frames | 1453248 |
| Max gap | 5407.722 ms |
| Gap histogram (normal <=250 ms / sparse <=1 s / gap >1 s) | 3029 / 0 / 1 |
| Time in sparse gaps | 0 s |
| Time in long gaps (>1 s) | 5.408 s |
| Drift at end | -264589 frames (-5512.271 ms) |
| Drift range | -265026 .. -258590 frames |
| Drift trend (stable window >10 s) | -259127.4 to -264589 frames; rate -762681.4 frames/h |
| Silent fraction (callbacks) | 1.29 % (39/3030) |

### Top 10 largest gaps

| t_rel_ms | gap_ms | class | silent |
| --- | --- | --- | --- |
| 5407.7 | 5407.722 | gap | false |
| 15542.9 | 84.588 | normal | false |
| 15630.4 | 42.467 | normal | false |
| 20992.6 | 34.801 | normal | false |
| 15417.5 | 19.561 | normal | false |
| 26847.9 | 19.349 | normal | false |
| 26987.4 | 19.197 | normal | false |
| 15451.3 | 13.966 | normal | false |
| 27918.9 | 11.499 | normal | false |
| 20348.8 | 11.451 | normal | false |

## Cross-source comparison

- Relative drift loopback vs microphone at end: -250513.3 frames (-5219.027 ms) after 35.8 s
- Relative drift rate: -25199537.6 frames/h
- Max gap microphone: 82.495 ms; loopback: 5407.722 ms

## Event log (scenario markers)

```text
2026-08-24 08:56:09.702+02:00 RUN_LAUNCH exe=C:\Users\haeni\Documents\Projekte\Speak system\src-tauri\target\debug\meeting_loopback_spike.exe duration_s=90
2026-08-24 08:56:09.726+02:00 RUN_STARTED pid=115004
2026-08-24 08:56:14.738+02:00 TONE_START t_rel_s=5
2026-08-24 08:56:45.168+02:00 TONE_END t_rel_s=35.5
2026-08-24 08:57:39.900+02:00 RUN_FINISHED exitcode=0
2026-08-24 08:57:39.904+02:00 ANALYZE_START
```

