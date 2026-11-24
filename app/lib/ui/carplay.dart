/// The CarPlay Driving Task screen's content (SPEC §9, BUILD_PLAN Step 10): a list with
/// the current alert, my speed against the limit and the nearest 5 vehicles, plus a mute
/// button. Built here, in English or Khmer from the ARB strings, and drawn by the iOS
/// CarPlay scene (`roadmesh/carplay` channel). The engine decided the alert; this only
/// words it.
library;

import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';

import '../l10n/app_localizations.dart';
import '../radio.dart';
import 'drive_screen.dart' show overLimit;
import 'style.dart';

/// Nearest vehicles shown in CarPlay (SPEC §9: 3 to 5).
const carPlayRows = 5;

/// One CarPlay screen state, as plain values for the platform channel.
@immutable
class CarPlayModel {
  /// Builds the model.
  const CarPlayModel({
    required this.title,
    required this.alert,
    required this.alertLevel,
    required this.speed,
    required this.overLimit,
    required this.nearestTitle,
    required this.rows,
    required this.muted,
    required this.muteLabel,
    required this.mutedNote,
  });

  /// "RoadMesh".
  final String title;

  /// Alert line (or "All clear").
  final String alert;

  /// "ok", "warning" or "danger".
  final String alertLevel;

  /// "48 km/h · limit 40".
  final String speed;

  /// More than 5 km/h over the limit.
  final bool overLimit;

  /// "Nearest vehicles".
  final String nearestTitle;

  /// (type, "36 km/h · 50 m", risk band name of the colour).
  final List<(String, String, String)> rows;

  /// Alerts muted.
  final bool muted;

  /// "Mute" or "Unmute".
  final String muteLabel;

  /// Shown under the alert when muted.
  final String mutedNote;

  /// For the channel.
  Map<String, Object> toMap() => {
    'title': title,
    'alert': alert,
    'alertLevel': alertLevel,
    'speed': speed,
    'overLimit': overLimit,
    'nearestTitle': nearestTitle,
    'rows': [
      for (final (t, d, b) in rows) {'text': t, 'detail': d, 'band': b},
    ],
    'muted': muted,
    'muteLabel': muteLabel,
    'mutedNote': mutedNote,
  };

  @override
  bool operator ==(Object other) =>
      other is CarPlayModel &&
      other.title == title &&
      other.alert == alert &&
      other.alertLevel == alertLevel &&
      other.speed == speed &&
      other.overLimit == overLimit &&
      other.nearestTitle == nearestTitle &&
      listEquals(other.rows, rows) &&
      other.muted == muted &&
      other.muteLabel == muteLabel &&
      other.mutedNote == mutedNote;

  @override
  int get hashCode => Object.hash(alert, speed, muted, Object.hashAll(rows));
}

/// The model for an update.
CarPlayModel carPlayModel(
  AppLocalizations l,
  RadioUpdate u, {
  required bool muted,
}) {
  Alert? top;
  for (final a in u.alerts) {
    if (a.level == 'warning' || a.level == 'danger') {
      top = a;
      break;
    }
  }
  final near = [...u.peers]..sort((a, b) => a.distance.compareTo(b.distance));
  final speed = kmh(u.own.speed);
  return CarPlayModel(
    title: l.appTitle,
    alert: top == null ? l.noAlert : alertText(l, top, u),
    alertLevel: top?.level ?? 'ok',
    speed: u.own.limit == null
        ? l.carplaySpeedNoLimit(speed)
        : l.carplaySpeed(speed, kmh(u.own.limit!)),
    overLimit: overLimit(u),
    nearestTitle: l.nearest,
    rows: [
      for (final p in near.take(carPlayRows))
        (
          typeName(l, p.vehicleType),
          l.carplayRow(kmh(p.speed), fmtDist(l, p.distance)),
          band(p.score).name,
        ),
    ],
    muted: muted,
    muteLabel: muted ? l.carplayUnmute : l.carplayMute,
    mutedNote: l.carplayMuted,
  );
}

/// Sends the model to the CarPlay scene when it changes, and hears the mute button.
class CarPlayLink {
  /// `onMute` is called with the new state when the driver taps Mute in the car.
  CarPlayLink({required this.onMute}) {
    _channel.setMethodCallHandler((call) async {
      if (call.method == 'mute') onMute(call.arguments == true);
      return null;
    });
  }

  static const _channel = MethodChannel('roadmesh/carplay');

  /// Mute button handler.
  final void Function(bool muted) onMute;

  CarPlayModel? _last;

  /// Send `m` if it differs from the last one sent.
  Future<void> show(CarPlayModel m) async {
    if (m == _last) return;
    _last = m;
    try {
      await _channel.invokeMethod('update', m.toMap());
    } on MissingPluginException {
      // Android and tests: no CarPlay
    }
  }
}
