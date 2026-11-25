// Store screenshots (AUDIT L-09, L-17; docs/STORE.md) in English and Khmer, from the real
// screens with sample traffic: iPhone 6.9" (1320 × 2868) and Android phone (1080 × 1920).
// Regenerate with `flutter test --update-goldens test/store_screenshots_test.dart`.
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:roadmesh_app/ui/controller.dart';

import 'support.dart';

void main() {
  setUpAll(loadFonts);
  setUp(() {
    fakeRadioChannel();
    useTolerantGoldens('test/store_screenshots_test.dart');
  });

  const devices = {
    'ios': Size(440, 956), // × 3 = 1320 × 2868
    'android': Size(360, 640), // × 3 = 1080 × 1920
  };
  final screens = <String, (double, void Function(DriveController))>{
    '1-drive': (32 / 3.6, (_) {}),
    '2-peers': (0, (c) => c.select(AppTab.peers)),
    '3-behaviour': (0, (c) => c.select(AppTab.behaviour)),
    '4-settings': (0, (c) => c.select(AppTab.settings)),
  };

  for (final lang in ['en', 'km']) {
    for (final d in devices.entries) {
      for (final s in screens.entries) {
        testWidgets('${d.key} ${s.key} $lang', (t) async {
          final c = controllerWith(sample(speed: s.value.$1), lang: lang, repeat: 40);
          await pumpApp(t, c, size: d.value, pixelRatio: 3);
          s.value.$2(c);
          await t.pump(const Duration(milliseconds: 100));
          await expectLater(
            find.byType(MaterialApp),
            matchesGoldenFile('../../docs/store/screenshots/${d.key}-${s.key}-$lang.png'),
          );
        });
      }
    }
  }
}
