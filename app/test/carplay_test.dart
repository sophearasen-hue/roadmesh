// CarPlay content (SPEC §9, BUILD_PLAN Step 10) and alert sounds.
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:roadmesh_app/l10n/app_localizations.dart';
import 'package:roadmesh_app/ui/carplay.dart';
import 'package:roadmesh_app/ui/controller.dart';
import 'package:shared_preferences/shared_preferences.dart';

import 'support.dart';

void main() {
  test(
    'CarPlay shows the alert, speed against the limit and the 5 nearest',
    () async {
      final en = await AppLocalizations.delegate.load(const Locale('en'));
      final m = carPlayModel(en, sample(speed: 48 / 3.6), muted: false);
      expect(
        m.alert,
        'Motorbike crossing from your right in 3.4 s — Brake firmly',
      );
      expect(m.alertLevel, 'danger');
      expect(m.speed, '48 km/h · limit 40');
      expect(m.overLimit, isTrue);
      expect(
        m.rows.length,
        4,
      ); // the sample has 4 vehicles; at most 5 are shown
      expect(m.rows.first, ('Motorbike', '36 km/h · 50 m', 'safe'));
      expect(m.rows[2].$3, 'risky');
      expect(m.muteLabel, 'Mute');
      final map = m.toMap();
      expect((map['rows']! as List).length, 4);
    },
  );

  test('CarPlay in Khmer, all clear, muted', () async {
    final km = await AppLocalizations.delegate.load(const Locale('km'));
    final m = carPlayModel(km, sample(alerts: const []), muted: true);
    expect(m.alertLevel, 'ok');
    expect(m.alert, km.noAlert);
    expect(m.muteLabel, km.carplayUnmute);
    expect(m.speed, contains('គ.ម/ម៉ោង'));
  });

  test('equal models compare equal (sent once)', () async {
    final en = await AppLocalizations.delegate.load(const Locale('en'));
    final a = carPlayModel(en, sample(), muted: false);
    final b = carPlayModel(en, sample(), muted: false);
    expect(a, b);
    expect(a == carPlayModel(en, sample(), muted: true), isFalse);
  });

  test('alert tone once per escalation, never when muted', () async {
    SharedPreferences.setMockInitialValues({});
    final c = DriveController(prefs: await SharedPreferences.getInstance());
    final played = <String>[];
    c.onAlertTone = played.add;
    final warn = sample(
      alerts: [
        {
          'kind': 'OwnSpeeding',
          'level': 'warning',
          'advice': 'EaseOff',
          'hops': 0,
        },
      ],
    );
    c.push(warn);
    c.push(warn); // still a warning: no repeat
    c.push(sample()); // danger: escalation
    c.push(sample());
    c.push(sample(alerts: const [])); // clear
    await c.setMuted(true);
    c.push(sample()); // muted
    expect(played, ['warning', 'danger']);
    expect(DriveController(prefs: c.prefs).muted, isTrue); // remembered
  });
}
