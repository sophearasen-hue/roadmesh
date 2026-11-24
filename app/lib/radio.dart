/// The RoadMesh radio on Android (plugin roadmesh_android) and iOS (roadmesh_ios):
/// both serve the same `roadmesh/radio` channels. Start/stop a drive and listen to what the
/// Rust safety engine decides. The UI shows this; it never decides alerts itself.
library;

import 'package:flutter/services.dart';

/// A vehicle heard by this phone, relative to me.
class Peer {
  /// Builds a peer from the service's map.
  Peer.fromMap(Map<Object?, Object?> m)
    : id = m['id'] as String? ?? '',
      east = (m['east'] as num?)?.toDouble() ?? 0,
      north = (m['north'] as num?)?.toDouble() ?? 0,
      distance = (m['distance'] as num?)?.toDouble() ?? 0,
      speed = (m['speed'] as num?)?.toDouble() ?? 0,
      heading = (m['heading'] as num?)?.toDouble() ?? 0,
      vehicleType = m['vehicleType'] as int? ?? 0,
      hops = m['hops'] as int? ?? 0,
      accuracy = (m['accuracy'] as num?)?.toDouble() ?? 0;

  /// Display key (changes with the pseudonym; never log it).
  final String id;

  /// Metres east / north of me.
  final double east, north;

  /// Distance (m).
  final double distance;

  /// Speed (m/s).
  final double speed;

  /// Compass heading (degrees).
  final double heading;

  /// Vehicle type code (0 car, 1 motorbike, 2 tuk-tuk, 3 truck, ...).
  final int vehicleType;

  /// 0 = heard directly.
  final int hops;

  /// Reported position accuracy (m).
  final double accuracy;
}

/// One alert; `kind` and `advice` are keys the app maps to its ARB strings.
class Alert {
  /// Builds an alert from the service's map.
  Alert.fromMap(Map<Object?, Object?> m)
    : kind = m['kind'] as String? ?? '',
      level = m['level'] as String? ?? 'caution',
      advice = m['advice'] as String? ?? 'None',
      advisedDecel = (m['advisedDecel'] as num?)?.toDouble() ?? 0,
      distance = (m['distance'] as num?)?.toDouble();

  /// Alert kind key, e.g. "CrossingConflict".
  final String kind;

  /// "caution", "warning" or "danger".
  final String level;

  /// Driver instruction key, e.g. "BrakeFirmly".
  final String advice;

  /// Advised deceleration (m/s²). Advice only.
  final double advisedDecel;

  /// Distance (m), if known.
  final double? distance;
}

/// One update from the drive service (5 per second).
class RadioUpdate {
  /// Alerts, most important first.
  final List<Alert> alerts;

  /// Vehicles heard.
  final List<Peer> peers;

  /// Builds an update from the service's map.
  RadioUpdate.fromMap(Map<Object?, Object?> m)
    : alerts = ((m['alerts'] as List?) ?? const [])
          .map((e) => Alert.fromMap(e as Map<Object?, Object?>))
          .toList(),
      peers = ((m['peers'] as List?) ?? const [])
          .map((e) => Peer.fromMap(e as Map<Object?, Object?>))
          .toList();
}

/// The radio and safety engine running natively (Android foreground service / iOS drive session).
class RoadMeshRadio {
  static const _methods = MethodChannel('roadmesh/radio');
  static const _events = EventChannel('roadmesh/radio/events');

  /// Ask for Bluetooth, location and notification permissions. True if all granted.
  static Future<bool> requestPermissions() async =>
      await _methods.invokeMethod<bool>('requestPermissions') ?? false;

  /// Start a drive. The notification texts come from the app's ARB strings.
  static Future<void> start({
    required String notificationTitle,
    required String notificationText,
    int vehicleType = 1,
  }) => _methods.invokeMethod('start', {
    'title': notificationTitle,
    'text': notificationText,
    'vehicleType': vehicleType,
  });

  /// Stop the drive.
  static Future<void> stop() => _methods.invokeMethod('stop');

  /// Radio and certificate status.
  static Future<Map<Object?, Object?>> status() async =>
      await _methods.invokeMethod<Map<Object?, Object?>>('status') ?? const {};

  /// Debug: write a certificate request for the developer CA; returns its path.
  static Future<String?> createCertificateRequest() =>
      _methods.invokeMethod<String>('createCertificateRequest');

  /// Debug: import the certificates the developer CA returned; returns how many are held.
  static Future<int?> importCertificates() =>
      _methods.invokeMethod<int>('importCertificates');

  /// Field test: write the CSV log; returns its path.
  static Future<String?> exportFieldLog() =>
      _methods.invokeMethod<String>('exportFieldLog');

  /// Decisions and peers while a drive runs.
  static Stream<RadioUpdate> get updates => _events
      .receiveBroadcastStream()
      .map((e) => RadioUpdate.fromMap(e as Map<Object?, Object?>));
}
