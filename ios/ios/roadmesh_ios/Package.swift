// swift-tools-version: 5.9
// RoadMesh iOS radio plugin (Swift Package Manager). Before building, run
// scripts/build-ios-core.sh on a Mac: it builds the Rust core as
// Frameworks/RoadMeshCoreFFI.xcframework and generates Sources/roadmesh_ios/RoadMeshCore.swift.
import PackageDescription

let package = Package(
    name: "roadmesh_ios",
    platforms: [.iOS("15.0")],
    products: [.library(name: "roadmesh-ios", targets: ["roadmesh_ios"])],
    dependencies: [
        .package(name: "FlutterFramework", path: "../FlutterFramework"),
        .package(name: "RoadMeshKit", path: "../../RoadMeshKit"),
    ],
    targets: [
        .binaryTarget(name: "RoadMeshCoreFFI", path: "Frameworks/RoadMeshCoreFFI.xcframework"),
        .target(
            name: "roadmesh_ios",
            dependencies: [
                .product(name: "FlutterFramework", package: "FlutterFramework"),
                .product(name: "RoadMeshKit", package: "RoadMeshKit"),
                "RoadMeshCoreFFI",
            ],
            resources: [.process("PrivacyInfo.xcprivacy")]
        ),
    ]
)
