// RoadMesh Android radio plugin. Before building, run scripts/build-android-core.sh:
// it cross-compiles the Rust core into src/main/jniLibs and generates its Kotlin
// bindings (package com.roadmesh.core) into src/main/kotlin.
group = "com.roadmesh.android"
version = "0.1.0"

buildscript {
    val kotlinVersion = "2.4.0"
    repositories {
        google()
        mavenCentral()
    }
    dependencies {
        classpath("com.android.tools.build:gradle:9.1.0")
        classpath("org.jetbrains.kotlin:kotlin-gradle-plugin:$kotlinVersion")
    }
}

allprojects {
    repositories {
        google()
        mavenCentral()
    }
}

plugins {
    id("com.android.library")
}

android {
    namespace = "com.roadmesh.android"
    compileSdk = 36

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    sourceSets {
        getByName("main") { java.srcDirs("src/main/kotlin") }
        getByName("test") { java.srcDirs("src/test/kotlin") }
    }

    defaultConfig {
        // Bluetooth LE extended advertising needs Android 8.0 (API 26)
        minSdk = 26
        consumerProguardFiles("consumer-rules.pro")
    }

    buildFeatures { buildConfig = true }

    buildTypes {
        all {
            // The edge server (docs/SERVER.md); empty values switch the cellular bridge off.
            fun prop(n: String) = ((project.findProperty(n) as String?) ?: "").replace("\"", "")
            buildConfigField("String", "SERVER_HOST", "\"${prop("roadmesh.serverHost")}\"")
            buildConfigField("String", "SERVER_PINS", "\"${prop("roadmesh.serverPins")}\"")
            buildConfigField("String", "CLOUD_PROJECT", "\"${prop("roadmesh.cloudProject")}\"")
            // The production pseudonym CA trust anchor (68 hex chars), printed by the server
            buildConfigField("String", "CA_ANCHOR", "\"${prop("roadmesh.caAnchor")}\"")
        }
        getByName("debug") {
            // Debug builds only: the public key of a developer CA (scripts/devca), passed as
            // -Proadmesh.devCaAnchor=<68 hex chars>. Release builds trust no test CA.
            val anchor = (project.findProperty("roadmesh.devCaAnchor") as String?) ?: ""
            buildConfigField("String", "DEV_CA_ANCHOR", "\"$anchor\"")
        }
        getByName("release") {
            buildConfigField("String", "DEV_CA_ANCHOR", "\"\"")
        }
    }

    testOptions {
        unitTests {
            all { it.useJUnitPlatform() }
        }
    }
}

kotlin {
    compilerOptions {
        jvmTarget = org.jetbrains.kotlin.gradle.dsl.JvmTarget.JVM_17
    }
}

dependencies {
    // UniFFI's Kotlin bindings call the Rust library through JNA
    implementation("net.java.dev.jna:jna:5.17.0@aar")
    // Play Integrity before certificates are issued (AUDIT S-16)
    implementation("com.google.android.play:integrity:1.4.0")
    testImplementation("org.jetbrains.kotlin:kotlin-test")
    testImplementation("org.junit.jupiter:junit-jupiter:5.12.2")
}
