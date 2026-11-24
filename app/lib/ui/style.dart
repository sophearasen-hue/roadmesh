/// Colours, text helpers and formatting shared by the screens. Matches
/// `reference/phone-prototype/index.html` (dark theme, risk bands, hop colours).
///
/// Presentation only: risk bands and labels turn the engine's numbers into colours
/// and words; no safety decision is made here.
library;

import 'dart:math' as math;

import 'package:flutter/material.dart';

import '../l10n/app_localizations.dart';
import '../radio.dart';

/// Prototype palette.
abstract final class C {
  static const bg = Color(0xFF0A0E13);
  static const panel = Color(0xFF111821);
  static const panel2 = Color(0xFF17212C);
  static const line = Color(0xFF243241);
  static const text = Color(0xFFE8EEF5);
  static const muted = Color(0xFF8A9BB0);
  static const blue = Color(0xFF3B82F6);
  static const safe = Color(0xFF22C55E);
  static const warn = Color(0xFFF59E0B);
  static const risk = Color(0xFFEF4444);
  static const tabOn = Color(0xFF93C5FD);
  static const unconnected = Color(0xFF3B4757);

  /// Mesh link colour by hop count (prototype HOPCOL).
  static const hops = [
    Color(0xFF60A5FA),
    Color(0xFF60A5FA),
    Color(0xFFA78BFA),
    Color(0xFFF472B6),
    Color(0xFFFB923C),
    Color(0xFFFACC15),
    Color(0xFF34D399),
  ];

  /// Vehicle icon tile colour by type code (prototype TCOL).
  static Color type(int code) => switch (code) {
    1 => const Color(0xFFFDA4AF),
    2 => const Color(0xFFFCD34D),
    3 || 4 => const Color(0xFFC4B5FD),
    _ => const Color(0xFF93C5FD),
  };
}

/// Prototype font: one bundled family for English and Khmer.
const fontFamily = 'Kantumruy Pro';

/// The app theme.
ThemeData roadMeshTheme() {
  final base = ThemeData(
    brightness: Brightness.dark,
    fontFamily: fontFamily,
    scaffoldBackgroundColor: C.panel,
    colorScheme: const ColorScheme.dark(
      primary: C.blue,
      surface: C.panel,
      onSurface: C.text,
    ),
  );
  return base.copyWith(
    textTheme: base.textTheme.apply(bodyColor: C.text, displayColor: C.text),
  );
}

/// Risk band of a 0–100 score (AUDIT P-02: Safe ≥85, Caution 65–84, Risky <65).
enum Band { safe, caution, risky }

/// Band of a score.
Band band(int score) => score >= 85
    ? Band.safe
    : score >= 65
    ? Band.caution
    : Band.risky;

/// Band colour.
Color bandColor(Band b) => switch (b) {
  Band.safe => C.safe,
  Band.caution => C.warn,
  Band.risky => C.risk,
};

/// Band name.
String bandName(AppLocalizations l, Band b) => switch (b) {
  Band.safe => l.riskSafe,
  Band.caution => l.riskCaution,
  Band.risky => l.riskRisky,
};

/// Vehicle type name.
String typeName(AppLocalizations l, int code) => switch (code) {
  0 => l.typeCar,
  1 => l.typeMoto,
  2 => l.typeTuk,
  3 => l.typeTruck,
  _ => l.typeOther,
};

/// Short tile label by type (prototype TLBL; Latin abbreviations in both languages).
String typeTag(int code) => switch (code) {
  0 => 'CAR',
  1 => 'MOTO',
  2 => 'TUK',
  3 => 'TRK',
  _ => 'VEH',
};

/// Distance: metres below 1 km, then km with one decimal.
String fmtDist(AppLocalizations l, double m) => m >= 1000
    ? '${(m / 1000).toStringAsFixed(1)} ${l.unitKm}'
    : '${m.round()} ${l.unitM}';

/// km/h from m/s.
int kmh(double mps) => (mps * 3.6).round();

/// Link description (P-13): direct, or relayed n times.
String linkText(AppLocalizations l, int hops) => hops <= 0
    ? l.direct
    : hops == 1
    ? l.hop1
    : l.viaHops(hops);

/// Behaviour event name by key.
String eventName(AppLocalizations l, String key) => switch (key) {
  'speeding' => l.evSpeeding,
  'hacc' => l.evHacc,
  'hbrake' => l.evHbrake,
  'overtake' => l.evOvertake,
  _ => l.evTailgating,
};

/// Behaviour event tag colours (prototype .tag-*): background, text.
(Color, Color) eventColors(String key) => switch (key) {
  'speeding' => (const Color(0x2EEF4444), const Color(0xFFFCA5A5)),
  'hbrake' => (const Color(0x2EF97316), const Color(0xFFFDBA74)),
  'hacc' => (const Color(0x2EF59E0B), const Color(0xFFFCD34D)),
  'overtake' => (const Color(0x338B5CF6), const Color(0xFFC4B5FD)),
  _ => (const Color(0x2E64748B), const Color(0xFFCBD5E1)),
};

/// The four behaviour event keys shown as chips.
const eventKeys = ['speeding', 'hacc', 'hbrake', 'overtake'];

/// Which side of my heading a peer is on (true = right). Compass degrees.
bool onMyRight(double myHeadingDeg, Peer p) {
  final bearing = math.atan2(p.east, p.north) * 180 / math.pi;
  final rel = ((bearing - myHeadingDeg + 540) % 360) - 180;
  return rel > 0;
}

/// The banner text for the engine's top alert (P-08, P-29 to P-33). The engine chose
/// the alert; this only words it, using the target's type and side for context.
String alertText(AppLocalizations l, Alert a, RadioUpdate u) {
  Peer? target;
  for (final p in u.peers) {
    if (p.id == a.target) target = p;
  }
  final type = target == null ? l.typeOther : typeName(l, target.vehicleType);
  final side = target == null
      ? l.sideRight
      : (onMyRight(u.own.heading, target) ? l.sideRight : l.sideLeft);
  final d = (a.distance ?? target?.distance ?? 0).round();
  final text = switch (a.kind) {
    'CrossingConflict' =>
      a.ttc == null
          ? l.alertCrossSoon(type, side)
          : l.alertCross(type, side, a.ttc!.toStringAsFixed(1)),
    'PriorityConflict' => l.alertPriority(type),
    'VehicleAhead' => l.alertVehicleAhead(type, d),
    'Oncoming' => l.alertOncoming(type),
    'NotYielding' => l.alertNotYielding(type),
    'LostContact' => l.alertLostContact,
    'HardBrakingAhead' => l.alertBrake(d),
    'HazardAhead' => l.alertHazard(d),
    'OvertakingYou' => l.alertOvertake(type),
    'FastApproachBehind' => l.alertBehind(type, kmh(target?.speed ?? 0)),
    'OwnSpeeding' => l.alertSpeed(
      math.max(0, kmh(u.own.speed) - kmh(u.own.limit ?? u.own.speed)),
    ),
    _ => l.alertPossible(type),
  };
  final withLink = a.hops > 0
      ? l.alertHazardRelayed(text, l.reachedBy(a.hops))
      : text;
  final advice = switch (a.advice) {
    'EaseOff' => l.adviceEaseOff,
    'BrakeGently' => l.adviceBrakeGently,
    'BrakeFirmly' => l.adviceBrakeFirmly,
    'BrakeHard' => l.adviceBrakeHard,
    _ => null,
  };
  return advice == null ? withLink : '$withLink — $advice';
}
