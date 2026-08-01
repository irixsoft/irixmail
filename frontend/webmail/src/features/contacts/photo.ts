import type { ContactPhoto } from "./types";

export const MAX_PHOTO_EDGE = 192;
export const MAX_PHOTO_BYTES = 100 * 1024;

const QUALITY_STEPS = [0.8, 0.65, 0.5, 0.35];
const DATA_URL_PATTERN = /^data:([^;,]+);base64,([\s\S]+)$/;

export function fitDimensions(width: number, height: number, max: number): { width: number; height: number } {
  const usable =
    Number.isFinite(width) && Number.isFinite(height) && Number.isFinite(max) && width > 0 && height > 0 && max > 0;
  if (!usable) return { width: 1, height: 1 };

  const longest = Math.max(width, height);
  const scale = longest > max ? max / longest : 1;
  return {
    width: Math.max(1, Math.round(width * scale)),
    height: Math.max(1, Math.round(height * scale)),
  };
}

export function base64Size(data: string): number {
  const clean = data.replace(/\s+/g, "");
  const padding = clean.endsWith("==") ? 2 : clean.endsWith("=") ? 1 : 0;
  return Math.floor(((clean.length - padding) * 3) / 4);
}

export function splitDataUrl(dataUrl: string): { mediaType: string; data: string } | null {
  const match = DATA_URL_PATTERN.exec(dataUrl.trim());
  if (!match) return null;
  return { mediaType: match[1]!, data: match[2]! };
}

export function photoSrc(photo: ContactPhoto): string {
  return `data:${photo.mediaType};base64,${photo.data}`;
}

export function nextQuality(quality: number): number | null {
  return QUALITY_STEPS.find((step) => step < quality - 1e-9) ?? null;
}

interface DecodedImage {
  source: CanvasImageSource;
  width: number;
  height: number;
  release: () => void;
}

async function decodeImage(file: File): Promise<DecodedImage> {
  if (typeof createImageBitmap === "function") {
    const bitmap = await createImageBitmap(file);
    return { source: bitmap, width: bitmap.width, height: bitmap.height, release: () => bitmap.close() };
  }

  const url = URL.createObjectURL(file);
  try {
    const image = await new Promise<HTMLImageElement>((resolve, reject) => {
      const element = new Image();
      element.onload = () => resolve(element);
      element.onerror = () => reject(new Error("That image could not be read"));
      element.src = url;
    });
    return {
      source: image,
      width: image.naturalWidth,
      height: image.naturalHeight,
      release: () => URL.revokeObjectURL(url),
    };
  } catch (error) {
    URL.revokeObjectURL(url);
    throw error;
  }
}

export async function fileToPhoto(file: File): Promise<ContactPhoto> {
  if (!file.type.startsWith("image/")) throw new Error("Pick an image file");

  const decoded = await decodeImage(file);
  try {
    const size = fitDimensions(decoded.width, decoded.height, MAX_PHOTO_EDGE);
    const canvas = document.createElement("canvas");
    canvas.width = size.width;
    canvas.height = size.height;
    const context = canvas.getContext("2d");
    if (!context) throw new Error("That image could not be read");
    context.drawImage(decoded.source, 0, 0, size.width, size.height);

    let quality: number | null = 0.8;
    while (quality !== null) {
      const encoded = splitDataUrl(canvas.toDataURL("image/jpeg", quality));
      if (encoded && base64Size(encoded.data) <= MAX_PHOTO_BYTES) return encoded;
      quality = nextQuality(quality);
    }
    throw new Error("That image is too large");
  } finally {
    decoded.release();
  }
}
