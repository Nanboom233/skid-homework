#include <cstddef>
#include <cstdint>
#include <cstdlib>
#include <cstring>
#include <new>
#include <vector>

#include <opencv2/core.hpp>
#include <opencv2/imgcodecs.hpp>
#include <opencv2/imgproc.hpp>

extern "C" {

int skid_enhance_scan(
    const std::uint8_t* input,
    std::size_t input_len,
    std::uint8_t** output,
    std::size_t* output_len) noexcept {
  if (output == nullptr || output_len == nullptr) {
    return 3;
  }

  *output = nullptr;
  *output_len = 0;

  if (input == nullptr || input_len == 0) {
    return 1;
  }

  try {
    const std::vector<std::uint8_t> source_bytes(input, input + input_len);
    const cv::Mat source = cv::imdecode(source_bytes, cv::IMREAD_COLOR);
    if (source.empty()) {
      return 2;
    }

    cv::Mat grayscale;
    cv::cvtColor(source, grayscale, cv::COLOR_BGR2GRAY);

    const cv::Mat kernel =
        cv::getStructuringElement(cv::MORPH_RECT, cv::Size(50, 50));

    cv::Mat background;
    cv::morphologyEx(grayscale, background, cv::MORPH_CLOSE, kernel);

    cv::Mat flattened;
    cv::divide(grayscale, background, flattened, 255.0, -1);

    cv::Mat thresholded;
    cv::threshold(
        flattened,
        thresholded,
        0.0,
        255.0,
        cv::THRESH_BINARY | cv::THRESH_OTSU);

    std::vector<std::uint8_t> encoded;
    if (!cv::imencode(".png", thresholded, encoded) || encoded.empty()) {
      return 4;
    }

    auto* result =
        static_cast<std::uint8_t*>(std::malloc(encoded.size()));
    if (result == nullptr) {
      return 5;
    }

    std::memcpy(result, encoded.data(), encoded.size());
    *output = result;
    *output_len = encoded.size();
    return 0;
  } catch (const std::bad_alloc&) {
    return 5;
  } catch (...) {
    return 3;
  }
}

void skid_enhance_scan_free(std::uint8_t* ptr) noexcept {
  std::free(ptr);
}

}
