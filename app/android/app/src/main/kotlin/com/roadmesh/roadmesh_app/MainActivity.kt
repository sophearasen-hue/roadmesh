package com.roadmesh.roadmesh_app

import android.os.Bundle
import io.flutter.embedding.android.FlutterActivity

class MainActivity : FlutterActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        // AUDIT S-12 (MobSF tapjacking): ignore touches while another app draws on top
        window.decorView.filterTouchesWhenObscured = true
    }
}
