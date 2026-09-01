import type { CapacitorConfig } from "@capacitor/cli";

const config: CapacitorConfig = {
  appId: "com.tulerws.lume.mobile",
  appName: "Lume",
  webDir: "mobile-pwa",
  loggingBehavior: "debug",
  server: {
    hostname: "localhost",
    androidScheme: "https",
    iosScheme: "capacitor",
    cleartext: true,
  },
  android: {
    allowMixedContent: true,
    backgroundColor: "#101713",
  },
  ios: {
    backgroundColor: "#101713",
    contentInset: "automatic",
  },
  plugins: {
    LocalNotifications: {
      smallIcon: "ic_lume_notification",
      iconColor: "#68B887",
    },
  },
};

export default config;
