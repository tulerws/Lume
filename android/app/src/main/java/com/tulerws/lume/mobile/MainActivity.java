package com.tulerws.lume.mobile;

import android.content.Intent;
import android.os.Bundle;
import com.getcapacitor.BridgeActivity;

public class MainActivity extends BridgeActivity {
    public static final String ACTION_OPEN_UPDATE =
        "com.tulerws.lume.mobile.action.OPEN_UPDATE";

    @Override
    public void onCreate(Bundle savedInstanceState) {
        registerPlugin(LumeNativePlugin.class);
        registerPlugin(LumeUpdaterPlugin.class);
        super.onCreate(savedInstanceState);
        UpdateCheckScheduler.schedule(this);
    }

    @Override
    protected void onNewIntent(Intent intent) {
        super.onNewIntent(intent);
        setIntent(intent);
    }
}
