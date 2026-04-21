# Native Scanner Models

Currently staged:

- `models/docaligner-fastvit_sa24.onnx`
  - Source: [DocsaidLab/DocAligner](https://github.com/DocsaidLab/DocAligner)
  - License: Apache-2.0 (see [LICENSE-docaligner](LICENSE-docaligner))
  - Task: document corner heatmap regression
  - Role: current public baseline for the desktop native backend

- UVDoc flatten model (used by `scanner_postprocess_model.rs`)
  - Source: [tanguymagne/UVDoc](https://github.com/tanguymagne/UVDoc)
  - License: MIT (see [LICENSE-uvdoc](LICENSE-uvdoc))
  - Task: neural grid-based document unwarping
  - Role: spine flattening via learned deformation grid

Planned primary model path:

- `models/document-boundary-yolo-pose.onnx`
  - Task: YOLO pose with 4 document corner keypoints
  - Role: long-term primary model after low-cost training/export

The Rust runtime now prefers the staged public baseline and reports the planned YOLO
slot separately so the two are not confused.
