/**
 * Processes an image using OpenCV.js to create a clean, high-contrast document.
 *
 * @param imageFile The original image file
 * @returns Promise with the processed File and Object URL
 */
export const processImage = async (
  imageFile: File,
): Promise<{ file: File; url: string }> => {
  return new Promise((resolve, reject) => {
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    const cv = (window as any).cv;

    if (!cv) {
      reject(new Error("OpenCV.js is not loaded"));
      return;
    }

    const waitForOpenCV = () => {
      if (cv.Mat) {
        run();
      } else {
        setTimeout(waitForOpenCV, 30);
      }
    };

    const run = () => {
      const img = new Image();
      const originalUrl = URL.createObjectURL(imageFile);

      img.onload = () => {
        // eslint-disable-next-line @typescript-eslint/no-explicit-any
        let src: any = null;
        // eslint-disable-next-line @typescript-eslint/no-explicit-any
        let dst: any = null;
        // eslint-disable-next-line @typescript-eslint/no-explicit-any
        let bg: any = null;
        // eslint-disable-next-line @typescript-eslint/no-explicit-any
        let kernelBig: any = null;
        // Note: kernelSmall is removed as it destroys digital text details

        try {
          src = cv.imread(img);
          dst = new cv.Mat();
          bg = new cv.Mat();

          // 1. Convert to Grayscale
          cv.cvtColor(src, src, cv.COLOR_RGBA2GRAY);

          // 2. Shadow Removal / Illumination Correction
          // Even for electronic docs, this helps normalize off-white backgrounds to pure white.
          kernelBig = cv.getStructuringElement(
            cv.MORPH_RECT,
            new cv.Size(50, 50),
          );

          // Estimate background
          cv.morphologyEx(src, bg, cv.MORPH_CLOSE, kernelBig);

          // Divide source by background to flatten illumination
          // dst = (src / bg) * 255
          cv.divide(src, bg, dst, 255, -1);

          // 4. Binarization (Thresholding)
          // For electronic documents (uniform lighting), Otsu's method works best.
          // It automatically finds the best separation between text and background.

          // Use THRESH_OTSU + THRESH_BINARY.
          // Note: When using OTSU, the explicit threshold value (0) is ignored.
          cv.threshold(dst, dst, 0, 255, cv.THRESH_BINARY | cv.THRESH_OTSU);

          // OPTIONAL: If you still need Adaptive Threshold (e.g., for mixed camera scans),
          // increase the Block Size (15 -> 31 or 41) to prevent letters from becoming hollow.
          /*
      cv.adaptiveThreshold(
        dst, dst, 255,
        cv.ADAPTIVE_THRESH_GAUSSIAN_C,
        cv.THRESH_BINARY,
        31, // Increased block size for better text preservation
        15  // Increased constant
      );
      */

          const outputCanvas = document.createElement("canvas");
          cv.imshow(outputCanvas, dst);

          outputCanvas.toBlob(
            (blob) => {
              if (!blob) {
                reject(new Error("Canvas encoding failed"));
                return;
              }

              const file = new File([blob], `enhanced_${imageFile.name}.png`, {
                type: "image/png",
              });

              resolve({
                file,
                url: URL.createObjectURL(file),
              });
            },
            "image/png",
            1,
          );
        } catch (err) {
          reject(err);
        } finally {
          // Clean up memory
          if (src) src.delete();
          if (dst) dst.delete();
          if (bg) bg.delete();
          if (kernelBig) kernelBig.delete();
          // kernelSmall was removed
          URL.revokeObjectURL(originalUrl);
        }
      };

      img.onerror = () => {
        URL.revokeObjectURL(originalUrl);
        reject(new Error("Image load failed"));
      };

      img.src = originalUrl;
    };

    waitForOpenCV();
  });
};
