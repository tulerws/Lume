package com.tulerws.lume.mobile;

import android.content.Intent;
import android.content.pm.PackageInfo;
import android.content.pm.PackageManager;
import android.net.Uri;
import android.os.Build;
import android.provider.Settings;
import androidx.core.content.FileProvider;
import com.getcapacitor.JSObject;
import com.getcapacitor.Plugin;
import com.getcapacitor.PluginCall;
import com.getcapacitor.PluginMethod;
import com.getcapacitor.annotation.CapacitorPlugin;
import java.io.File;
import java.io.FileOutputStream;
import java.io.InputStream;
import java.net.HttpURLConnection;
import java.net.URI;
import java.net.URL;
import java.security.MessageDigest;
import java.util.Arrays;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;

@CapacitorPlugin(name = "LumeUpdater")
public class LumeUpdaterPlugin extends Plugin {
    private static final long MAX_APK_BYTES = 200L * 1024L * 1024L;
    private final ExecutorService executor = Executors.newSingleThreadExecutor();

    @Override
    protected void handleOnDestroy() {
        executor.shutdownNow();
        super.handleOnDestroy();
    }

    @PluginMethod
    public void getInfo(PluginCall call) {
        try {
            PackageInfo info = installedPackageInfo();
            JSObject result = new JSObject();
            result.put("version", info.versionName);
            result.put("versionCode", versionCode(info));
            result.put("canInstallUpdates", canInstallUpdates());
            call.resolve(result);
        } catch (Exception error) {
            call.reject("Could not read the installed Lume version.", "VERSION_READ_FAILED", error);
        }
    }

    @PluginMethod
    public void installUpdate(PluginCall call) {
        String url = call.getString("url", "");
        String expectedSha256 = call.getString("sha256", "");
        if (!isTrustedReleaseUrl(url) || !expectedSha256.matches("(?i)^[a-f0-9]{64}$")) {
            call.reject("The update metadata is invalid.", "INVALID_UPDATE");
            return;
        }

        if (!canInstallUpdates()) {
            Intent settings = new Intent(
                Settings.ACTION_MANAGE_UNKNOWN_APP_SOURCES,
                Uri.parse("package:" + getContext().getPackageName())
            );
            getBridge().executeOnMainThread(() -> getActivity().startActivity(settings));
            call.reject(
                "Allow Lume to install updates, then return and try again.",
                "INSTALL_PERMISSION_REQUIRED"
            );
            return;
        }

        executor.submit(() -> {
            File updateFile = null;
            try {
                updateFile = downloadApk(url, expectedSha256);
                validateApk(updateFile);
                File installableUpdate = updateFile;
                getBridge().executeOnMainThread(() -> {
                    try {
                        openPackageInstaller(installableUpdate);
                        call.resolve();
                    } catch (Exception error) {
                        call.reject("Could not open the Android installer.", "INSTALLER_FAILED", error);
                    }
                });
            } catch (Exception error) {
                if (updateFile != null) updateFile.delete();
                call.reject(error.getMessage(), "UPDATE_DOWNLOAD_FAILED", error);
            }
        });
    }

    private PackageInfo installedPackageInfo() throws PackageManager.NameNotFoundException {
        int flags = Build.VERSION.SDK_INT >= Build.VERSION_CODES.P
            ? PackageManager.GET_SIGNING_CERTIFICATES
            : PackageManager.GET_SIGNATURES;
        return getContext().getPackageManager().getPackageInfo(getContext().getPackageName(), flags);
    }

    private boolean canInstallUpdates() {
        return Build.VERSION.SDK_INT < Build.VERSION_CODES.O ||
            getContext().getPackageManager().canRequestPackageInstalls();
    }

    private boolean isTrustedReleaseUrl(String value) {
        try {
            URI uri = URI.create(value);
            return "https".equalsIgnoreCase(uri.getScheme()) &&
                "github.com".equalsIgnoreCase(uri.getHost()) &&
                uri.getPath().startsWith("/tulerws/Lume/releases/download/");
        } catch (Exception ignored) {
            return false;
        }
    }

    private File downloadApk(String source, String expectedSha256) throws Exception {
        File directory = new File(getContext().getCacheDir(), "updates");
        if (!directory.exists() && !directory.mkdirs()) {
            throw new IllegalStateException("Could not prepare the update directory.");
        }
        File destination = new File(directory, "lume-mobile-update.apk");
        File temporary = new File(directory, "lume-mobile-update.apk.part");
        destination.delete();
        temporary.delete();

        HttpURLConnection connection = (HttpURLConnection) new URL(source).openConnection();
        connection.setConnectTimeout(15_000);
        connection.setReadTimeout(30_000);
        connection.setInstanceFollowRedirects(true);
        connection.setRequestProperty("User-Agent", "Lume-Mobile-Updater");
        connection.connect();
        if (connection.getResponseCode() / 100 != 2) {
            connection.disconnect();
            throw new IllegalStateException("The update download returned an unexpected response.");
        }
        long declaredSize = connection.getContentLengthLong();
        if (declaredSize > MAX_APK_BYTES) {
            connection.disconnect();
            throw new IllegalStateException("The update is larger than the allowed limit.");
        }

        MessageDigest digest = MessageDigest.getInstance("SHA-256");
        long downloaded = 0;
        try (
            InputStream input = connection.getInputStream();
            FileOutputStream output = new FileOutputStream(temporary)
        ) {
            byte[] buffer = new byte[32 * 1024];
            int count;
            while ((count = input.read(buffer)) != -1) {
                downloaded += count;
                if (downloaded > MAX_APK_BYTES) {
                    throw new IllegalStateException("The update is larger than the allowed limit.");
                }
                output.write(buffer, 0, count);
                digest.update(buffer, 0, count);
            }
        } finally {
            connection.disconnect();
        }

        String actualSha256 = hexadecimal(digest.digest());
        if (!actualSha256.equalsIgnoreCase(expectedSha256)) {
            temporary.delete();
            throw new SecurityException("The downloaded update did not pass integrity verification.");
        }
        if (!temporary.renameTo(destination)) {
            throw new IllegalStateException("Could not prepare the downloaded update.");
        }
        return destination;
    }

    private void validateApk(File apk) throws Exception {
        PackageManager manager = getContext().getPackageManager();
        int flags = Build.VERSION.SDK_INT >= Build.VERSION_CODES.P
            ? PackageManager.GET_SIGNING_CERTIFICATES
            : PackageManager.GET_SIGNATURES;
        PackageInfo candidate = manager.getPackageArchiveInfo(apk.getAbsolutePath(), flags);
        PackageInfo installed = installedPackageInfo();
        if (candidate == null || !getContext().getPackageName().equals(candidate.packageName)) {
            throw new SecurityException("The downloaded file is not a Lume update.");
        }
        if (versionCode(candidate) <= versionCode(installed)) {
            throw new IllegalStateException("This Lume version is already installed.");
        }
        if (!signaturesMatch(installed, candidate)) {
            throw new SecurityException("The update signature does not match the installed application.");
        }
    }

    @SuppressWarnings("deprecation")
    private boolean signaturesMatch(PackageInfo installed, PackageInfo candidate) {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.P) {
            if (installed.signingInfo == null || candidate.signingInfo == null) return false;
            return Arrays.equals(
                installed.signingInfo.getApkContentsSigners(),
                candidate.signingInfo.getApkContentsSigners()
            );
        }
        return Arrays.equals(installed.signatures, candidate.signatures);
    }

    @SuppressWarnings("deprecation")
    private long versionCode(PackageInfo info) {
        return Build.VERSION.SDK_INT >= Build.VERSION_CODES.P
            ? info.getLongVersionCode()
            : info.versionCode;
    }

    private void openPackageInstaller(File apk) {
        Uri uri = FileProvider.getUriForFile(
            getContext(),
            getContext().getPackageName() + ".fileprovider",
            apk
        );
        Intent intent = new Intent(Intent.ACTION_VIEW);
        intent.setDataAndType(uri, "application/vnd.android.package-archive");
        intent.addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION | Intent.FLAG_ACTIVITY_NEW_TASK);
        getContext().startActivity(intent);
    }

    private String hexadecimal(byte[] bytes) {
        StringBuilder value = new StringBuilder(bytes.length * 2);
        for (byte current : bytes) value.append(String.format("%02x", current));
        return value.toString();
    }
}
