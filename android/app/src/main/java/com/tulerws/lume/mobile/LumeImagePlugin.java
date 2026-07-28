package com.tulerws.lume.mobile;

import android.content.ClipData;
import android.content.ClipDescription;
import android.content.ClipboardManager;
import android.graphics.Bitmap;
import android.graphics.BitmapFactory;
import android.graphics.Matrix;
import android.media.ExifInterface;
import android.net.Uri;
import android.util.Base64;
import com.getcapacitor.JSObject;
import com.getcapacitor.Plugin;
import com.getcapacitor.PluginCall;
import com.getcapacitor.PluginMethod;
import com.getcapacitor.annotation.CapacitorPlugin;
import java.io.ByteArrayInputStream;
import java.io.ByteArrayOutputStream;
import java.io.InputStream;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;

@CapacitorPlugin(name = "LumeImages")
public class LumeImagePlugin extends Plugin {
    private static final int MAX_INPUT_BYTES = 20 * 1024 * 1024;
    private static final int MAX_TRANSFER_BYTES = 1_350_000;
    private final ExecutorService executor = Executors.newSingleThreadExecutor();

    @Override
    protected void handleOnDestroy() {
        executor.shutdownNow();
        super.handleOnDestroy();
    }

    @PluginMethod
    public void prepareImage(PluginCall call) {
        String encoded = call.getString("dataBase64", "");
        if (encoded.isEmpty() || encoded.length() > ((MAX_INPUT_BYTES * 4L) / 3L) + 8) {
            call.reject("This image is empty or larger than 20 MB.", "INVALID_IMAGE");
            return;
        }
        executor.submit(() -> {
            try {
                byte[] input = Base64.decode(encoded, Base64.DEFAULT);
                if (input.length == 0 || input.length > MAX_INPUT_BYTES) {
                    throw new IllegalArgumentException("This image is empty or larger than 20 MB.");
                }
                PreparedImage prepared = prepare(input);
                JSObject result = new JSObject();
                result.put("mimeType", "image/jpeg");
                result.put("dataBase64", Base64.encodeToString(prepared.full, Base64.NO_WRAP));
                result.put(
                    "previewDataUrl",
                    "data:image/jpeg;base64," + Base64.encodeToString(prepared.preview, Base64.NO_WRAP)
                );
                call.resolve(result);
            } catch (OutOfMemoryError error) {
                rejectImage(call, new Exception(error));
            } catch (Exception error) {
                rejectImage(call, error);
            }
        });
    }

    @PluginMethod
    public void readClipboardImage(PluginCall call) {
        ClipboardManager clipboard =
            (ClipboardManager) getContext().getSystemService(android.content.Context.CLIPBOARD_SERVICE);
        ClipData clip = clipboard == null ? null : clipboard.getPrimaryClip();
        ClipDescription description = clipboard == null ? null : clipboard.getPrimaryClipDescription();
        if (clip == null || clip.getItemCount() == 0) {
            call.reject("The clipboard does not contain an image.", "NO_CLIPBOARD_IMAGE");
            return;
        }
        ClipData.Item item = clip.getItemAt(0);
        Uri uri = item.getUri();
        if (uri == null && item.getIntent() != null) uri = item.getIntent().getData();
        String resolvedType = uri == null ? null : getContext().getContentResolver().getType(uri);
        boolean containsImage =
            (description != null && description.hasMimeType("image/*"))
            || (resolvedType != null && resolvedType.startsWith("image/"));
        if (uri == null || !containsImage) {
            call.reject("The clipboard image is unavailable.", "NO_CLIPBOARD_IMAGE");
            return;
        }
        Uri imageUri = uri;
        executor.submit(() -> {
            try {
                byte[] input = readClipboardBytes(imageUri);
                PreparedImage prepared = prepare(input);
                JSObject result = new JSObject();
                result.put("name", "clipboard-image.jpg");
                result.put("mimeType", "image/jpeg");
                result.put("dataBase64", Base64.encodeToString(prepared.full, Base64.NO_WRAP));
                result.put(
                    "previewDataUrl",
                    "data:image/jpeg;base64," + Base64.encodeToString(prepared.preview, Base64.NO_WRAP)
                );
                call.resolve(result);
            } catch (Exception error) {
                rejectImage(call, error);
            }
        });
    }

    private byte[] readClipboardBytes(Uri uri) throws Exception {
        try (InputStream input = getContext().getContentResolver().openInputStream(uri)) {
            if (input == null) throw new IllegalArgumentException("Clipboard image is unavailable.");
            ByteArrayOutputStream output = new ByteArrayOutputStream();
            byte[] buffer = new byte[16 * 1024];
            int total = 0;
            int read;
            while ((read = input.read(buffer)) != -1) {
                total += read;
                if (total > MAX_INPUT_BYTES) {
                    throw new IllegalArgumentException("The clipboard image is larger than 20 MB.");
                }
                output.write(buffer, 0, read);
            }
            return output.toByteArray();
        }
    }

    private void rejectImage(PluginCall call, Exception error) {
        call.reject(
            "Could not read this image. Try a JPEG, PNG, WebP, GIF or HEIC file under 20 MB.",
            "IMAGE_DECODE_FAILED",
            error
        );
    }

    private PreparedImage prepare(byte[] input) throws Exception {
        BitmapFactory.Options bounds = new BitmapFactory.Options();
        bounds.inJustDecodeBounds = true;
        BitmapFactory.decodeByteArray(input, 0, input.length, bounds);
        if (bounds.outWidth <= 0 || bounds.outHeight <= 0) {
            throw new IllegalArgumentException("Unsupported image.");
        }

        int sample = 1;
        while (Math.max(bounds.outWidth, bounds.outHeight) / (sample * 2) >= 1_600) {
            sample *= 2;
        }
        BitmapFactory.Options options = new BitmapFactory.Options();
        options.inSampleSize = sample;
        options.inPreferredConfig = Bitmap.Config.ARGB_8888;
        Bitmap decoded = BitmapFactory.decodeByteArray(input, 0, input.length, options);
        if (decoded == null) throw new IllegalArgumentException("Unsupported image.");

        Bitmap oriented = orient(decoded, input);
        try {
            byte[] full = encodeWithinLimit(oriented);
            Bitmap previewBitmap = scaled(oriented, 360);
            try {
                byte[] preview = encode(previewBitmap, 68);
                return new PreparedImage(full, preview);
            } finally {
                if (previewBitmap != oriented) previewBitmap.recycle();
            }
        } finally {
            if (oriented != decoded) oriented.recycle();
            decoded.recycle();
        }
    }

    private byte[] encodeWithinLimit(Bitmap source) {
        for (int dimension : new int[] { 1_600, 1_400, 1_200, 960 }) {
            Bitmap candidate = scaled(source, dimension);
            try {
                for (int quality : new int[] { 82, 74, 68, 60 }) {
                    byte[] encoded = encode(candidate, quality);
                    if (encoded.length <= MAX_TRANSFER_BYTES) return encoded;
                }
            } finally {
                if (candidate != source) candidate.recycle();
            }
        }
        throw new IllegalArgumentException("This image could not be prepared for secure transfer.");
    }

    private byte[] encode(Bitmap bitmap, int quality) {
        ByteArrayOutputStream output = new ByteArrayOutputStream();
        if (!bitmap.compress(Bitmap.CompressFormat.JPEG, quality, output)) {
            throw new IllegalArgumentException("Could not encode image.");
        }
        return output.toByteArray();
    }

    private Bitmap scaled(Bitmap source, int maxDimension) {
        int largest = Math.max(source.getWidth(), source.getHeight());
        if (largest <= maxDimension) return source;
        float scale = maxDimension / (float) largest;
        int width = Math.max(1, Math.round(source.getWidth() * scale));
        int height = Math.max(1, Math.round(source.getHeight() * scale));
        return Bitmap.createScaledBitmap(source, width, height, true);
    }

    private Bitmap orient(Bitmap source, byte[] input) {
        try {
            int orientation = new ExifInterface(new ByteArrayInputStream(input))
                .getAttributeInt(ExifInterface.TAG_ORIENTATION, ExifInterface.ORIENTATION_NORMAL);
            Matrix matrix = new Matrix();
            switch (orientation) {
                case ExifInterface.ORIENTATION_FLIP_HORIZONTAL -> matrix.setScale(-1, 1);
                case ExifInterface.ORIENTATION_ROTATE_180 -> matrix.setRotate(180);
                case ExifInterface.ORIENTATION_FLIP_VERTICAL -> matrix.setScale(1, -1);
                case ExifInterface.ORIENTATION_TRANSPOSE -> {
                    matrix.setRotate(90);
                    matrix.postScale(-1, 1);
                }
                case ExifInterface.ORIENTATION_ROTATE_90 -> matrix.setRotate(90);
                case ExifInterface.ORIENTATION_TRANSVERSE -> {
                    matrix.setRotate(-90);
                    matrix.postScale(-1, 1);
                }
                case ExifInterface.ORIENTATION_ROTATE_270 -> matrix.setRotate(-90);
                default -> {
                    return source;
                }
            }
            return Bitmap.createBitmap(
                source,
                0,
                0,
                source.getWidth(),
                source.getHeight(),
                matrix,
                true
            );
        } catch (Exception ignored) {
            return source;
        }
    }

    private static final class PreparedImage {
        final byte[] full;
        final byte[] preview;

        PreparedImage(byte[] full, byte[] preview) {
            this.full = full;
            this.preview = preview;
        }
    }
}
