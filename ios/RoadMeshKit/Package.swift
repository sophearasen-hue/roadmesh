// swift-tools-version: 5.9
// Pure Swift helpers of the RoadMesh iOS radio (no Flutter, no hardware), unit-tested with
// `swift test`. The plugin in ../ios/roadmesh_ios depends on this package.
import PackageDescription

let package = Package(
    name: "RoadMeshKit",
    platforms: [.iOS("15.0"), .macOS("12.0")],
    products: [.library(name: "RoadMeshKit", targets: ["RoadMeshKit"])],
    targets: [
        .target(name: "RoadMeshKit"),
        .testTarget(name: "RoadMeshKitTests", dependencies: ["RoadMeshKit"]),
    ]
)
