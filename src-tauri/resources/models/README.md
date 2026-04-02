# Native Scanner Models

Currently staged:

- `models/docaligner-fastvit_sa24.onnx`
  - Source: DocsaidLab DocAligner
  - Task: document corner heatmap regression
  - Role: current public baseline for the desktop native backend

Planned primary model path:

- `models/document-boundary-yolo-pose.onnx`
  - Task: YOLO pose with 4 document corner keypoints
  - Role: long-term primary model after low-cost training/export

The Rust runtime now prefers the staged public baseline and reports the planned YOLO
slot separately so the two are not confused.
