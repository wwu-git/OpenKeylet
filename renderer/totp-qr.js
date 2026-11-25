async function decodeTotpQrImage(file) {
  if (!file.size || file.size > 10 * 1024 * 1024) return null;
  if (file.type && !/^image\/(png|jpeg|webp|gif|bmp|tiff)$/.test(file.type)) return null;

  const image = await createImageBitmap(file);
  try {
    const scale = Math.min(1, 1536 / Math.max(image.width, image.height));
    const canvas = document.createElement('canvas');
    canvas.width = Math.max(1, Math.round(image.width * scale));
    canvas.height = Math.max(1, Math.round(image.height * scale));
    const context = canvas.getContext('2d', { willReadFrequently: true });
    context.fillStyle = 'white';
    context.fillRect(0, 0, canvas.width, canvas.height);
    context.drawImage(image, 0, 0, canvas.width, canvas.height);
    return window.OpenKeyletQRDecoder.decode(
      context.getImageData(0, 0, canvas.width, canvas.height),
    )?.trim() || null;
  } finally {
    image.close();
  }
}
