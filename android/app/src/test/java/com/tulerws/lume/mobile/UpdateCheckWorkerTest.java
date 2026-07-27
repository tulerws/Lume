package com.tulerws.lume.mobile;

import static org.junit.Assert.assertFalse;
import static org.junit.Assert.assertTrue;

import org.junit.Test;

public class UpdateCheckWorkerTest {
    @Test
    public void detectsNewerSemanticVersions() {
        assertTrue(UpdateCheckWorker.isVersionNewer("0.6.1", "0.6.0"));
        assertTrue(UpdateCheckWorker.isVersionNewer("0.7.0", "0.6.12"));
        assertTrue(UpdateCheckWorker.isVersionNewer("1.0.0", "0.99.99"));
    }

    @Test
    public void ignoresInstalledOrOlderVersions() {
        assertFalse(UpdateCheckWorker.isVersionNewer("0.6.0", "0.6.0"));
        assertFalse(UpdateCheckWorker.isVersionNewer("0.5.9", "0.6.0"));
        assertFalse(UpdateCheckWorker.isVersionNewer("0.6.0-beta.1", "0.6.0"));
    }
}
