# VCR playback noise reduction

## What the references describe

- JVC's VTG82063 Section 3 describes HQ YNR as a recursive comb filter built from 1H delay elements. The playback circuit makes a low-pass reference from correlated scanlines, extracts the difference, limits it to protect edges, and subtracts the noise component. The guide explicitly calls out the tradeoff: unprotected vertical averaging softens edges, so the limiter preserves vertical detail.
- The same guide's digital-video section describes synchronized sampling and field memory. It gives 10.7 MHz and 6–7 bit as common home-video values, and describes a Y/C-separated implementation using 6-bit digitization with separate Y and colour memories. The 6-bit ADC discussion is an example architecture, not a claim that every high-end VCR used the same converter.
- Toshiba's 1995 3DNR paper describes a one-frame DRAM delay, cyclic comparison of current and delayed Y, and similar processing on demodulated colour differences. It says motion adaptation is needed to avoid residual images. Its implementation used 8-bit samples and a 2 Mbit DRAM, showing that the exact precision and memory varied by product and generation.
- JVC's later HR-S9500 technical guide describes 3DYNR as frame-cyclic luminance NR, with Hadamard conversion and dynamic-vector correction to reduce residual images. The reduction amount was selectable, and NR was disabled in digital trick-play modes.
- The supplied Section 5 is about VCR microprocessors, mechacon and timer control. It explains device control and serial data transfer, but does not specify the pixel-level YNR/DNR algorithm.

## Implementation in this simulator

The controls are in **再生 NR (高級 VCR)**. Both strengths default to zero, so existing output stays unchanged until one is enabled.

- **YNR** is a recursive low-pass along field scanlines (the 1H-correlated direction) applied to recovered luminance. A soft edge gate and a bounded correction preserve strong vertical edges.
- **Digital 3D-DNR** compares Y, U and V with the same field parity from two fields earlier (one frame). A cross-shaped low-pass of the luma difference estimates motion; temporal averaging fades out as measured motion rises. Filtered results are fed back into field memory for cyclic noise reduction.
- **NR ADC/DAC bit depth** quantizes the Y/C planes on both sides of the digital processing. It defaults to 6 bits based on the JVC example, supports 0–10 bits, and uses 0 for no quantization. The model maps Y to -40..120 IRE and U/V to ±0.5 white-level units before quantizing. It does not reproduce the source guide's 10.7 MHz sampling clock or its exact memory wiring.

This is a software model of the published signal path, not a cycle-accurate copy of a particular VCR's IC. In particular, motion detection approximates the cited Hadamard/vector-correction approach with a local luma-difference measure, and stronger temporal settings can still leave trails around fast motion. The current pipeline applies this filter at the recovered Y/C boundary in the decoder; when post-tape RF is enabled, that RF noise is also downstream of NR and can be reduced.

## References

- JVC, *Video Technical Guide VTG82063*, [Section 3 — Video Circuit](https://decode-orc.github.io/analogue-video-specifications/vhs/JVC-Video-Technical-Guide-VTG82063/section-3/), especially 3.3.6, 3.4 and 3.6.
- JVC, *Video Technical Guide VTG82063*, [Section 4 — Servo Circuit](https://decode-orc.github.io/analogue-video-specifications/vhs/JVC-Video-Technical-Guide-VTG82063/section-4/), YNR pulse timing.
- JVC, *Video Technical Guide VTG82063*, [Section 5 — Microprocessor Systems](https://decode-orc.github.io/analogue-video-specifications/vhs/JVC-Video-Technical-Guide-VTG82063/section-5/), included to distinguish control firmware from the video NR datapath.
- M. Takahashi, “Three-Dimensional Digital Noise Reduction System for Home Hi-Fi VCRs,” *Toshiba Review* 50(8), 1995, [article PDF](https://www.global.toshiba/content/dam/toshiba/migration/corp/techReviewAssets/tech/review/1995/08/50_08pdf/f03.pdf).
- JVC, *HR-S9500 Video Technical Guide* (1999), [technical guide transcription](https://studylib.net/doc/25695121/jvc-hr-s9500u-english), Section 2.2.2, playback-mode video processor.
