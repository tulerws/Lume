package com.tulerws.lume.mobile;

import android.os.Bundle;
import com.getcapacitor.BridgeActivity;

public class MainActivity extends BridgeActivity {
    @Override
    public void onCreate(Bundle savedInstanceState) {
        registerPlugin(LumeUpdaterPlugin.class);
        super.onCreate(savedInstanceState);
    }
}
