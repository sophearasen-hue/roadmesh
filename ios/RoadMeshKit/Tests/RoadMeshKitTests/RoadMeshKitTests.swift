import XCTest
@testable import RoadMeshKit

final class RoadMeshKitTests: XCTestCase {
    func testDerToRaw() {
        // SEQUENCE { INTEGER 0x00ff…(33 bytes, leading zero), INTEGER 0x01 }
        let r = [UInt8](repeating: 0xff, count: 32)
        var der: [UInt8] = [0x30, 0x00, 0x02, 33, 0x00] + r + [0x02, 0x01, 0x01]
        der[1] = UInt8(der.count - 2)
        let raw = Crypto.derToRaw(Data(der))
        XCTAssertEqual(raw?.count, 64)
        XCTAssertEqual(Array(raw!.prefix(32)), r)
        XCTAssertEqual(Array(raw!.suffix(32)), [UInt8](repeating: 0, count: 31) + [1])
        XCTAssertNil(Crypto.derToRaw(Data([0x31, 0x00])))
        XCTAssertNil(Crypto.derToRaw(Data()))
    }

    func testCompress() {
        var p = [UInt8](repeating: 7, count: 65)
        p[0] = 0x04
        p[64] = 0x01
        XCTAssertEqual(Crypto.compress(uncompressed: Data(p))?.first, 0x03)
        p[64] = 0x02
        XCTAssertEqual(Crypto.compress(uncompressed: Data(p))?.count, 33)
        XCTAssertEqual(Crypto.compress(uncompressed: Data(p))?.first, 0x02)
        XCTAssertNil(Crypto.compress(uncompressed: Data([1, 2])))
    }

    func testHex() {
        XCTAssertEqual(Crypto.hex(Data([0, 1, 255, 16])), "0001ff10")
        XCTAssertEqual(Crypto.unhex("0001ff10"), Data([0, 1, 255, 16]))
        XCTAssertNil(Crypto.unhex("abc"))
    }

    func testFieldLogMatchesAndroidFormat() {
        let log = FieldLog()
        for _ in 0..<60 {
            for _ in 0..<9 { log.onAccepted(pseudonymHex: "deadbeef", rssi: -70) }
            log.everySecond(["deadbeef": 120])
        }
        log.everyMinute(batteryPct: 87)
        let lines = log.csv().split(separator: "\n")
        XCTAssertEqual(lines.count, 2)
        XCTAssertEqual(lines[1], "0,87,100-150,540,600,90.0,-70")
        XCTAssertFalse(log.csv().contains("deadbeef"))
    }

    func testFieldLogClearForgetsEverything() {
        let log = FieldLog()
        log.everySecond(["deadbeef": 20])
        log.everyMinute(batteryPct: 50)
        log.clear()
        XCTAssertEqual(log.csv().split(separator: "\n").count, 1)
    }

    func testPinningMatchesOnlyPinnedKeys() {
        let a = Data([0x04] + [UInt8](repeating: 1, count: 64))
        let b = Data([0x04] + [UInt8](repeating: 2, count: 64))
        let pins = [Pinning.sha256(Pinning.spki(p256Point: a)!), Data(repeating: 9, count: 32)]
        XCTAssertTrue(Pinning.matches(p256Point: a, pins: pins))
        XCTAssertFalse(Pinning.matches(p256Point: b, pins: pins))
        XCTAssertFalse(Pinning.matches(p256Point: Data([1, 2, 3]), pins: pins))
        // the SPKI is the standard 91-byte DER structure
        XCTAssertEqual(Pinning.spki(p256Point: a)?.count, 91)
    }

    func testPinListNeedsABackupPin() {
        let one = Data(repeating: 1, count: 32).base64EncodedString()
        let two = Data(repeating: 2, count: 32).base64EncodedString()
        XCTAssertNil(Pinning.parse(one))
        XCTAssertEqual(Pinning.parse("\(one), \(two)")?.count, 2)
        XCTAssertNil(Pinning.parse("\(one),not-base64"))
        XCTAssertNil(Pinning.parse(""))
    }
}
