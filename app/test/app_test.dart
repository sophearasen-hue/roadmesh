// Widget tests for AUDIT A1–A6 screen items (P-01 to P-28, P-40 to P-43).
import 'dart:convert';
import 'dart:io';

import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:roadmesh_app/radio.dart';
import 'package:roadmesh_app/ui/controller.dart';
import 'package:roadmesh_app/ui/map_view.dart';
import 'package:roadmesh_app/ui/style.dart';
import 'package:shared_preferences/shared_preferences.dart';

import 'support.dart';

void main() {
  setUp(fakeRadioChannel);

  testWidgets(
    'P-08 Danger banner words the engine alert with side, time and advice',
    (t) async {
      await pumpApp(t, controllerWith(sample()));
      expect(find.byKey(const Key('alert-danger')), findsOneWidget);
      // peer is east-south-east of me while I head east: on my right
      expect(
        find.text('Motorbike crossing from your right in 3.4 s — Brake firmly'),
        findsOneWidget,
      );
    },
  );

  testWidgets(
    'P-08 no Warning or Danger: green all clear (Caution only rings the map)',
    (t) async {
      await pumpApp(
        t,
        controllerWith(
          sample(
            alerts: [
              {
                'kind': 'PossibleConflict',
                'level': 'caution',
                'advice': 'None',
                'target': 'aa110001',
              },
            ],
          ),
        ),
      );
      expect(find.byKey(const Key('alert-ok')), findsOneWidget);
      expect(find.text('All clear — no hazards detected'), findsOneWidget);
    },
  );

  testWidgets(
    'P-08 Warning banner is amber; hazard heard through relays says so',
    (t) async {
      await pumpApp(
        t,
        controllerWith(
          sample(
            alerts: [
              {
                'kind': 'HazardAhead',
                'level': 'warning',
                'advice': 'BrakeGently',
                'distance': 1281.0,
                'hops': 5,
              },
            ],
          ),
        ),
      );
      expect(find.byKey(const Key('alert-warning')), findsOneWidget);
      expect(
        find.text(
          'Stopped traffic 1281 m ahead · Reached via 5 relays — Brake gently',
        ),
        findsOneWidget,
      );
    },
  );

  testWidgets(
    'P-09 HUD: speed, limit sign pulsing when over, peers, reach, score',
    (t) async {
      await pumpApp(t, controllerWith(sample(speed: 50 / 3.6)));
      expect(find.text('50'), findsWidgets);
      expect(find.byKey(const Key('limit-over')), findsOneWidget);
      expect(t.widget<Text>(find.byKey(const Key('peersCount'))).data, '4');
      expect(t.widget<Text>(find.byKey(const Key('reach'))).data, '275 m');
      expect(t.widget<Text>(find.byKey(const Key('score'))).data, '81');
    },
  );

  test('P-02 risk bands: Safe ≥85, Caution 65–84, Risky <65', () {
    expect([100, 85, 84, 65, 64, 0].map(band).toList(), [
      Band.safe,
      Band.safe,
      Band.caution,
      Band.caution,
      Band.risky,
      Band.risky,
    ]);
  });

  test('P-05 mesh links: each relayed peer links to the nearest peer one hop closer', () {
    final u = sample();
    final parents = meshParents(u.peers);
    expect(parents['aa110001'], isNull); // direct: linked to me
    expect(parents['cc330003']?.id, 'aa110001');
    expect(parents['dd440004']?.id, 'cc330003');
  });

  testWidgets('P-06 Nearby ↔ Network view toggle', (t) async {
    final c = controllerWith(sample());
    await pumpApp(t, c);
    expect(find.text('Network view'), findsOneWidget);
    await t.tap(find.byKey(const Key('viewToggle')));
    await t.pump(const Duration(milliseconds: 50));
    expect(c.networkView, isTrue);
    expect(find.text('Nearby view'), findsOneWidget);
  });

  testWidgets('P-07 legend', (t) async {
    await pumpApp(t, controllerWith(sample()));
    for (final s in ['You', 'Safe', 'Caution', 'Risky']) {
      expect(find.text(s), findsOneWidget);
    }
  });

  testWidgets('P-10 tapping a peer on the map opens its detail', (t) async {
    final c = controllerWith(sample());
    await pumpApp(t, c);
    final box = t.getRect(find.byKey(const Key('map')));
    final g = MapGeometry(box.size, c.last!.peers, network: false);
    await t.tapAt(box.topLeft + g.at(-60, 20));
    await t.pump(const Duration(milliseconds: 50));
    expect(c.selected, 'bb220002');
    expect(find.byKey(const Key('peerDetail')), findsOneWidget);
  });

  testWidgets('P-11 release builds have no driver demo controls', (t) async {
    final c = controllerWith(sample());
    await pumpApp(t, c);
    // no demo attached: no speed −/+, hard brake or hazard demo, in any build
    for (final k in ['slower', 'faster', 'brake', 'demoBtn']) {
      expect(find.byKey(Key(k)), findsNothing);
    }
    expect(kReleaseMode || c.demo == null, isTrue);
  });

  testWidgets(
    'P-12/P-13 peers list: title, sorted by distance, link, badge, speed',
    (t) async {
      final c = controllerWith(sample());
      await pumpApp(t, c);
      await t.tap(find.byKey(const Key('tab-peers')));
      await t.pump(const Duration(milliseconds: 50));
      expect(find.text('4 vehicles in your mesh'), findsOneWidget);
      final ys = [
        for (final id in ['aa110001', 'bb220002', 'cc330003'])
          t.getTopLeft(find.byKey(Key('peer-$id'))).dy,
      ];
      expect(ys, orderedEquals([...ys]..sort()));
      expect(find.textContaining('50 m · Direct'), findsOneWidget);
      expect(find.textContaining('150 m · 1 hop'), findsOneWidget);
      expect(find.text('Risky · 55'), findsOneWidget);
      expect(
        find.textContaining('RM-CC33', findRichText: true),
        findsOneWidget,
      );
    },
  );

  testWidgets('P-14 empty state', (t) async {
    await pumpApp(t, controllerWith(RadioUpdate.fromMap(const {})));
    await t.tap(find.byKey(const Key('tab-peers')));
    await t.pump(const Duration(milliseconds: 50));
    expect(find.textContaining('No vehicles in range'), findsOneWidget);
  });

  testWidgets('P-15 peer detail: values, graph, chips, events; back', (
    t,
  ) async {
    final c = controllerWith(sample(), repeat: 10);
    await pumpApp(t, c);
    c.select(AppTab.peers);
    c.openPeer('bb220002');
    await t.pump(const Duration(milliseconds: 50));
    expect(find.text('43 km/h'), findsOneWidget);
    expect(find.text('-0.4 m/s²'), findsOneWidget);
    expect(find.text('60 ms'), findsNothing); // direct: 30 ms
    expect(find.text('30 ms'), findsOneWidget);
    expect(find.text('Harsh braking · 2'), findsOneWidget);
    expect(find.text('Speed, last 2 minutes'), findsOneWidget);
    expect(find.textContaining('Car RM-BB22 · 41 km/h'), findsOneWidget);
    await t.tap(find.byKey(const Key('back')));
    await t.pump(const Duration(milliseconds: 50));
    expect(find.text('4 vehicles in your mesh'), findsOneWidget);
  });

  testWidgets('P-16 screens other than Drive are locked above 10 km/h', (
    t,
  ) async {
    final c = controllerWith(sample());
    await pumpApp(t, c);
    expect(c.select(AppTab.behaviour), isTrue);
    c.push(sample(speed: 11 / 3.6));
    await t.pump(const Duration(milliseconds: 50));
    expect(c.tab, AppTab.drive);
    await t.tap(find.byKey(const Key('tab-settings')));
    await t.pump(const Duration(milliseconds: 50));
    expect(c.tab, AppTab.drive);
    expect(find.text('Available when you slow below 10 km/h'), findsOneWidget);
    c.push(sample(speed: 9 / 3.6));
    expect(c.select(AppTab.settings), isTrue);
  });

  testWidgets('P-17 to P-22 behaviour tab', (t) async {
    final c = controllerWith(sample(), repeat: 6);
    await pumpApp(t, c);
    await t.tap(find.byKey(const Key('tab-behaviour')));
    await t.pump(const Duration(milliseconds: 50));
    expect(find.byKey(const Key('scoreRing')), findsOneWidget);
    expect(find.text('Caution'), findsWidgets); // 81 = Caution band name
    expect(find.text('12.4 km'), findsOneWidget);
    expect(find.text('32 km/h'), findsOneWidget); // average
    expect(find.text('59 km/h'), findsOneWidget); // top
    expect(find.text('7'), findsOneWidget); // total events
    expect(find.text('Overtake · 3'), findsOneWidget);
    expect(find.text('1 risky · 1 caution · 2 safe'), findsOneWidget);
    await t.drag(find.byType(ListView), const Offset(0, -600));
    await t.pump(const Duration(milliseconds: 50));
    expect(find.text('12s ago'), findsOneWidget);
    await t.tap(find.byKey(const Key('f-me')));
    await t.pump(const Duration(milliseconds: 50));
    expect(find.text('12s ago'), findsNothing);
    expect(find.textContaining('You · 52 km/h'), findsOneWidget);
    await t.tap(find.byKey(const Key('f-peers')));
    await t.pump(const Duration(milliseconds: 50));
    expect(find.textContaining('You ·'), findsNothing);
  });

  testWidgets('P-23 language applies at once and is remembered', (t) async {
    SharedPreferences.setMockInitialValues({});
    final prefs = await SharedPreferences.getInstance();
    final c = DriveController(prefs: prefs)..push(sample());
    await pumpApp(t, c);
    await t.tap(find.byKey(const Key('langPill')));
    await t.pump(const Duration(milliseconds: 50));
    expect(find.text('បើកបរ'), findsOneWidget);
    expect(prefs.getString('language'), 'km');
    expect(DriveController(prefs: prefs).locale, const Locale('km'));
  });

  testWidgets('P-24/P-25/P-26/P-28 settings', (t) async {
    SharedPreferences.setMockInitialValues({});
    final prefs = await SharedPreferences.getInstance();
    final c = DriveController(prefs: prefs)..locale = const Locale('en');
    await pumpApp(t, c);
    c.select(AppTab.settings);
    await t.pump(const Duration(milliseconds: 50));
    expect(
      find.text('Phone only · ~120 m per hop · 30 ms per hop'),
      findsOneWidget,
    );
    expect(find.text('Long-range radio add-on'), findsOneWidget);
    // the hop limit is a debug setting only (tests run in debug mode)
    expect(
      find.byKey(const Key('hops')),
      kDebugMode ? findsOneWidget : findsNothing,
    );
    await t.drag(find.byType(ListView), const Offset(0, -500));
    await t.pump(const Duration(milliseconds: 50));
    expect(
      find.textContaining('random ID that changes every 5 minutes'),
      findsOneWidget,
    );
    await t.ensureVisible(find.byKey(const Key('moto')));
    await t.pump(const Duration(milliseconds: 50));
    await t.tap(find.byKey(const Key('moto')));
    await t.pump(const Duration(milliseconds: 50));
    expect(prefs.getBool('motorbikeMode'), isTrue);
  });

  testWidgets(
    'P-27 delete my data asks, then clears the service and the screens',
    (t) async {
      var deleted = 0;
      final c = DriveController(onDeleteData: () async => deleted++)
        ..locale = const Locale('en')
        ..push(sample());
      await pumpApp(t, c);
      c.select(AppTab.settings);
      await t.pump(const Duration(milliseconds: 50));
      await t.drag(find.byType(ListView), const Offset(0, -900));
      await t.pump(const Duration(milliseconds: 50));
      await t.ensureVisible(find.byKey(const Key('deleteData')));
      await t.pump(const Duration(milliseconds: 50));
      await t.tap(find.byKey(const Key('deleteData')));
      await t.pump(const Duration(milliseconds: 300));
      await t.tap(find.byKey(const Key('confirmDelete')));
      await t.pump(const Duration(milliseconds: 300));
      expect(deleted, 1);
      expect(c.last, isNull);
      expect(find.text('Your data was deleted.'), findsOneWidget);
    },
  );

  test('P-40 every key in app_en.arb is in app_km.arb and no extra ones', () {
    Map<String, dynamic> arb(String f) =>
        jsonDecode(File('lib/l10n/$f').readAsStringSync())
            as Map<String, dynamic>;
    final en = arb('app_en.arb').keys.where((k) => !k.startsWith('@')).toSet();
    final km = arb('app_km.arb').keys.where((k) => !k.startsWith('@')).toSet();
    expect(km, en);
    expect(arb('app_km.arb')['@@x-needs-review'], isTrue);
  });

  test('P-40 no hard-coded user text in the screens', () {
    // Text widgets must take ARB strings; allowed literals: language names, symbols,
    // units shared by both languages and the Latin vehicle tags.
    final allowed = {'English', 'ខ្មែរ', 'EN', '—', '● ', ' m/s²', 'm/s²'};
    final literal = RegExp(r"""Text\(\s*'([^'$]+)'""");
    for (final f in Directory('lib/ui').listSync().whereType<File>()) {
      for (final m in literal.allMatches(f.readAsStringSync())) {
        expect(
          allowed,
          contains(m.group(1)),
          reason: '${f.path}: ${m.group(1)}',
        );
      }
    }
  });
}
