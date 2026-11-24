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
      accel = (m['accel'] as num?)?.toDouble() ?? 0,
      heading = (m['heading'] as num?)?.toDouble() ?? 0,
      vehicleType = m['vehicleType'] as int? ?? 0,
      hops = m['hops'] as int? ?? 0,
      accuracy = (m['accuracy'] as num?)?.toDouble() ?? 0,
      ageMs = m['ageMs'] as int? ?? 0,
      score = m['score'] as int? ?? 100,
      counts = EventCounts.fromMap(m['counts']);

  /// Display key (changes with the pseudonym; never log it).
  final String id;

  /// Metres east / north of me.
  final double east, north;

  /// Distance (m).
  final double distance;

  /// Speed (m/s).
  final double speed;

  /// Reported acceleration (m/s²).
  final double accel;

  /// Compass heading (degrees).
  final double heading;

  /// Vehicle type code (0 car, 1 motorbike, 2 tuk-tuk, 3 truck, ...).
  final int vehicleType;

  /// 0 = heard directly.
  final int hops;

  /// Reported position accuracy (m).
  final double accuracy;

  /// Age of its latest message (ms).
  final int ageMs;

  /// Driving score 0–100 observed by this phone.
  final int score;

  /// Behaviour episodes observed while it was in my mesh.
  final EventCounts counts;
}

/// Behaviour episode counts.
class EventCounts {
  /// Builds counts from the service's map (missing = zero).
  EventCounts.fromMap(Object? o)
    : speeding = _int(o, 'speeding'),
      hacc = _int(o, 'hacc'),
      hbrake = _int(o, 'hbrake'),
      overtake = _int(o, 'overtake');

  /// Counts by event key ("speeding", "hacc", "hbrake", "overtake").
  final int speeding, hacc, hbrake, overtake;

  /// Count for an event key.
  int of(String key) => switch (key) {
    'speeding' => speeding,
    'hacc' => hacc,
    'hbrake' => hbrake,
    'overtake' => overtake,
    _ => 0,
  };

  /// Sum of all counts.
  int get total => speeding + hacc + hbrake + overtake;

  static int _int(Object? o, String k) =>
      o is Map ? (o[k] as num?)?.toInt() ?? 0 : 0;
}

/// This phone's own state as the engine used it.
class OwnState {
  /// Builds the own state from the service's map.
  OwnState.fromMap(Object? o)
    : speed = _d(o, 'speed') ?? 0,
      heading = _d(o, 'heading') ?? 0,
      accuracy = _d(o, 'accuracy') ?? 0,
      limit = _d(o, 'limit');

  /// Speed (m/s).
  final double speed;

  /// Compass heading (degrees).
  final double heading;

  /// GPS accuracy (m).
  final double accuracy;

  /// Speed limit here (m/s), if the map knows it.
  final double? limit;

  static double? _d(Object? o, String k) =>
      o is Map ? (o[k] as num?)?.toDouble() : null;
}

/// This phone's own trip.
class Trip {
  /// Builds the trip from the service's map.
  Trip.fromMap(Object? o)
    : distance = OwnState._d(o, 'distance') ?? 0,
      avgSpeed = OwnState._d(o, 'avgSpeed') ?? 0,
      maxSpeed = OwnState._d(o, 'maxSpeed') ?? 0,
      score = o is Map ? (o['score'] as num?)?.toInt() ?? 100 : 100,
      counts = EventCounts.fromMap(o is Map ? o['counts'] : null);

  /// Distance (m).
  final double distance;

  /// Average and top speed (m/s).
  final double avgSpeed, maxSpeed;

  /// Driving score 0–100.
  final int score;

  /// Episode counts.
  final EventCounts counts;
}

/// One behaviour event for the timeline.
class BehaviourEvent {
  /// Builds an event from the service's map.
  BehaviourEvent.fromMap(Map<Object?, Object?> m)
    : age = (m['age'] as num?)?.toDouble() ?? 0,
      kind = m['kind'] as String? ?? '',
      me = m['me'] as bool? ?? false,
      id = m['id'] as String? ?? '',
      kmh = (m['kmh'] as num?)?.toDouble() ?? 0;

  /// Seconds ago.
  final double age;

  /// "speeding", "hacc", "hbrake", "overtake" or "tailgating".
  final String kind;

  /// My own vehicle.
  final bool me;

  /// The peer's display key (empty for me).
  final String id;

  /// Speed at the time (km/h).
  final double kmh;
}

/// One alert; `kind` and `advice` are keys the app maps to its ARB strings.
class Alert {
  /// Builds an alert from the service's map.
  Alert.fromMap(Map<Object?, Object?> m)
    : kind = m['kind'] as String? ?? '',
      level = m['level'] as String? ?? 'caution',
      advice = m['advice'] as String? ?? 'None',
      advisedDecel = (m['advisedDecel'] as num?)?.toDouble() ?? 0,
      distance = (m['distance'] as num?)?.toDouble(),
      ttc = (m['ttc'] as num?)?.toDouble(),
      hops = m['hops'] as int? ?? 0,
      target = m['target'] as String?;

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

  /// Predicted time to conflict (s), if known.
  final double? ttc;

  /// Relays the information came through.
  final int hops;

  /// Display key of the other vehicle, if any.
  final String? target;
}

/// One update from the drive service (5 per second).
class RadioUpdate {
  /// Alerts, most important first.
  final List<Alert> alerts;

  /// Vehicles heard.
  final List<Peer> peers;

  /// My own state.
  final OwnState own;

  /// My trip so far.
  final Trip trip;

  /// Newest behaviour events first.
  final List<BehaviourEvent> events;

  /// Builds an update from the service's map.
  RadioUpdate.fromMap(Map<Object?, Object?> m)
    : alerts = ((m['alerts'] as List?) ?? const [])
          .map((e) => Alert.fromMap(e as Map<Object?, Object?>))
          .toList(),
      peers = ((m['peers'] as List?) ?? const [])
          .map((e) => Peer.fromMap(e as Map<Object?, Object?>))
          .toList(),
      own = OwnState.fromMap(m['own']),
      trip = Trip.fromMap(m['trip']),
      events = ((m['events'] as List?) ?? const [])
          .map((e) => BehaviourEvent.fromMap(e as Map<Object?, Object?>))
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

  /// Play the alert tone for "warning" or "danger" (native sound; respects the ringer).
  static Future<void> alertTone(String level) async {
    try {
      await _methods.invokeMethod('alertTone', {'level': level});
    } on MissingPluginException {
      // tests
    }
  }

  /// "Delete my data" on this phone.
  static Future<void> deleteMyData() => _methods.invokeMethod('deleteMyData');

  /// Field test: write the CSV log; returns its path.
  static Future<String?> exportFieldLog() =>
      _methods.invokeMethod<String>('exportFieldLog');

  /// Decisions and peers while a drive runs.
  static Stream<RadioUpdate> get updates => _events
      .receiveBroadcastStream()
      .map((e) => RadioUpdate.fromMap(e as Map<Object?, Object?>));
}
