import Foundation

/// Jailbreak detection (AUDIT S-12). A jailbroken phone could have its keys or messages
/// tampered with, so it **lowers trust** instead of blocking the driver: the phone keeps
/// receiving and warning, but sends nothing (no broadcast, no uploads). The server also
/// refuses certificates to devices that fail App Attest.
enum DeviceTrust {
    private static let paths = [
        "/Applications/Cydia.app", "/Applications/Sileo.app", "/Library/MobileSubstrate/MobileSubstrate.dylib",
        "/bin/bash", "/usr/sbin/sshd", "/etc/apt", "/private/var/lib/apt/", "/var/jb", "/usr/bin/ssh",
    ]

    static var compromised: Bool {
        #if targetEnvironment(simulator)
        return false
        #else
        if paths.contains(where: { FileManager.default.fileExists(atPath: $0) }) { return true }
        // a sandboxed app cannot write outside its container
        let probe = "/private/roadmesh-probe-\(UUID().uuidString)"
        if (try? "x".write(toFile: probe, atomically: true, encoding: .utf8)) != nil {
            try? FileManager.default.removeItem(atPath: probe)
            return true
        }
        return false
        #endif
    }
}
