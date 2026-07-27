package com.tulerws.lume.mobile;

import android.Manifest;
import android.app.NotificationChannel;
import android.app.NotificationManager;
import android.app.PendingIntent;
import android.content.Context;
import android.content.Intent;
import android.content.SharedPreferences;
import android.content.pm.PackageInfo;
import android.content.pm.PackageManager;
import android.os.Build;
import androidx.annotation.NonNull;
import androidx.core.app.NotificationCompat;
import androidx.core.app.NotificationManagerCompat;
import androidx.core.content.ContextCompat;
import androidx.work.Worker;
import androidx.work.WorkerParameters;
import java.io.ByteArrayOutputStream;
import java.io.InputStream;
import java.net.HttpURLConnection;
import java.net.URI;
import java.net.URL;
import java.nio.charset.StandardCharsets;
import org.json.JSONObject;

public final class UpdateCheckWorker extends Worker {
    private static final String MANIFEST_URL =
        "https://github.com/tulerws/Lume/releases/latest/download/mobile-latest.json";
    private static final String CHANNEL_ID = "lume-app-updates";
    private static final String PREFERENCES = "lume-native-update-checks";
    private static final String LAST_NOTIFIED_VERSION = "last-notified-version";
    private static final int NOTIFICATION_ID = 6100;
    private static final int MAX_MANIFEST_BYTES = 64 * 1024;

    public UpdateCheckWorker(
        @NonNull Context context,
        @NonNull WorkerParameters parameters
    ) {
        super(context, parameters);
    }

    @NonNull
    @Override
    public Result doWork() {
        try {
            UpdateManifest manifest = fetchManifest();
            String installedVersion = installedVersion();
            if (!isVersionNewer(manifest.version, installedVersion)) return Result.success();

            SharedPreferences preferences = getApplicationContext().getSharedPreferences(
                PREFERENCES,
                Context.MODE_PRIVATE
            );
            if (manifest.version.equals(preferences.getString(LAST_NOTIFIED_VERSION, ""))) {
                return Result.success();
            }
            if (showUpdateNotification(manifest.version)) {
                preferences.edit().putString(LAST_NOTIFIED_VERSION, manifest.version).apply();
            }
            return Result.success();
        } catch (Exception error) {
            return getRunAttemptCount() < 3 ? Result.retry() : Result.failure();
        }
    }

    static boolean isVersionNewer(String candidate, String installed) {
        int[] candidateParts = versionParts(candidate);
        int[] installedParts = versionParts(installed);
        for (int index = 0; index < candidateParts.length; index += 1) {
            if (candidateParts[index] != installedParts[index]) {
                return candidateParts[index] > installedParts[index];
            }
        }
        return false;
    }

    private static int[] versionParts(String value) {
        String stable = String.valueOf(value).split("-", 2)[0];
        String[] parts = stable.split("\\.");
        int[] parsed = new int[4];
        for (int index = 0; index < parsed.length && index < parts.length; index += 1) {
            String numeric = parts[index].replaceFirst("[^0-9].*$", "");
            try {
                parsed[index] = numeric.isEmpty() ? 0 : Integer.parseInt(numeric);
            } catch (NumberFormatException ignored) {
                parsed[index] = 0;
            }
        }
        return parsed;
    }

    static UpdateManifest fetchManifest() throws Exception {
        HttpURLConnection connection = (HttpURLConnection) new URL(MANIFEST_URL).openConnection();
        connection.setConnectTimeout(15_000);
        connection.setReadTimeout(20_000);
        connection.setInstanceFollowRedirects(true);
        connection.setRequestProperty("Accept", "application/json");
        connection.setRequestProperty("User-Agent", "Lume-Mobile-Update-Check");
        try {
            if (connection.getResponseCode() / 100 != 2) {
                throw new IllegalStateException("Update manifest request failed.");
            }
            try (
                InputStream input = connection.getInputStream();
                ByteArrayOutputStream output = new ByteArrayOutputStream()
            ) {
                byte[] buffer = new byte[4096];
                int total = 0;
                int count;
                while ((count = input.read(buffer)) != -1) {
                    total += count;
                    if (total > MAX_MANIFEST_BYTES) {
                        throw new IllegalStateException("Update manifest is too large.");
                    }
                    output.write(buffer, 0, count);
                }
                return parseManifest(output.toString(StandardCharsets.UTF_8.name()));
            }
        } finally {
            connection.disconnect();
        }
    }

    static UpdateManifest parseManifest(String payload) throws Exception {
        JSONObject root = new JSONObject(payload);
        JSONObject android = root.getJSONObject("android");
        String version = root.getString("version");
        String apkUrl = android.getString("url");
        String sha256 = android.getString("sha256");
        return validateManifest(version, apkUrl, sha256);
    }

    static UpdateManifest validateManifest(String version, String apkUrl, String sha256) {
        if (
            version.trim().isEmpty()
            || !isTrustedReleaseUrl(apkUrl)
            || !sha256.matches("(?i)^[a-f0-9]{64}$")
        ) {
            throw new SecurityException("Update manifest is invalid.");
        }
        return new UpdateManifest(version, apkUrl, sha256);
    }

    private static boolean isTrustedReleaseUrl(String value) {
        try {
            URI uri = URI.create(value);
            return "https".equalsIgnoreCase(uri.getScheme())
                && "github.com".equalsIgnoreCase(uri.getHost())
                && uri.getPath().startsWith("/tulerws/Lume/releases/download/");
        } catch (Exception ignored) {
            return false;
        }
    }

    private String installedVersion() throws PackageManager.NameNotFoundException {
        PackageInfo info = getApplicationContext().getPackageManager().getPackageInfo(
            getApplicationContext().getPackageName(),
            0
        );
        return info.versionName == null ? "0.0.0" : info.versionName;
    }

    private boolean showUpdateNotification(String version) {
        Context context = getApplicationContext();
        if (
            Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU
            && ContextCompat.checkSelfPermission(context, Manifest.permission.POST_NOTIFICATIONS)
                != PackageManager.PERMISSION_GRANTED
        ) {
            return false;
        }
        NotificationManager manager = context.getSystemService(NotificationManager.class);
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            NotificationChannel channel = new NotificationChannel(
                CHANNEL_ID,
                "Lume updates",
                NotificationManager.IMPORTANCE_DEFAULT
            );
            channel.setDescription("New Lume Mobile versions");
            manager.createNotificationChannel(channel);
        }

        Intent intent = new Intent(context, MainActivity.class)
            .setAction(MainActivity.ACTION_OPEN_UPDATE)
            .addFlags(Intent.FLAG_ACTIVITY_CLEAR_TOP | Intent.FLAG_ACTIVITY_SINGLE_TOP);
        PendingIntent pendingIntent = PendingIntent.getActivity(
            context,
            NOTIFICATION_ID,
            intent,
            PendingIntent.FLAG_UPDATE_CURRENT | PendingIntent.FLAG_IMMUTABLE
        );
        NotificationCompat.Builder notification = new NotificationCompat.Builder(context, CHANNEL_ID)
            .setSmallIcon(android.R.drawable.stat_sys_download_done)
            .setContentTitle("Lume " + version + " is available")
            .setContentText("Tap to review and install the update.")
            .setStyle(new NotificationCompat.BigTextStyle()
                .bigText("A new signed Lume Mobile version is ready. Tap to review and install it without removing the current app."))
            .setContentIntent(pendingIntent)
            .setAutoCancel(true)
            .setOnlyAlertOnce(true)
            .setPriority(NotificationCompat.PRIORITY_DEFAULT);
        NotificationManagerCompat.from(context).notify(NOTIFICATION_ID, notification.build());
        return true;
    }

    static final class UpdateManifest {
        final String version;
        final String apkUrl;
        final String sha256;

        UpdateManifest(String version, String apkUrl, String sha256) {
            this.version = version;
            this.apkUrl = apkUrl;
            this.sha256 = sha256;
        }
    }
}
