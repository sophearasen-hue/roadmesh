// swift-tools-version: 5.9
// Unit tests for the pure Swift helpers of the RoadMesh iOS radio (no Flutter, no hardware):
// `swift test`. Sources/RoadMeshKit is a link to ../ios/roadmesh_ios/Sources/RoadMeshKit,
// where the plugin builds them as its own target.
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
