// Golden screenshots of every screen in English and Khmer at 360 px width (AUDIT P-42:
// no Khmer overflow on a 360-px phone; any RenderFlex overflow fails the test) and the
// CarPlay-sized layout. Regenerate with `flutter test --update-goldens test/golden_test.dart`.
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:roadmesh_app/ui/controller.dart';

import 'support.dart';

void main() {
  setUpAll(loadFonts);
  setUp(() {
    fakeRadioChannel();
    useTolerantGoldens('test/golden_test.dart');
  });

  for (final lang in ['en', 'km']) {
    Future<DriveController> shot(
      WidgetTester t,
      String name,
      void Function(DriveController c) open, {
      Size size = const Size(360, 760),
      double speed = 0,
    }) async {
      final c = controllerWith(sample(speed: speed), lang: lang, repeat: 40);
      await pumpApp(t, c, size: size);
      open(c);
      await t.pump(const Duration(milliseconds: 100));
      await expectLater(
        find.byType(MaterialApp),
        matchesGoldenFile('goldens/$name-$lang.png'),
      );
      return c;
    }

    testWidgets(
      'drive $lang',
      (t) => shot(t, 'drive', (_) {}, speed: 48 / 3.6),
    );
    testWidgets(
      'network view $lang',
      (t) => shot(t, 'network', (c) => c.toggleView()),
    );
    testWidgets(
      'peers $lang',
      (t) => shot(t, 'peers', (c) => c.select(AppTab.peers)),
    );
    testWidgets(
      'peer detail $lang',
      (t) => shot(t, 'peer-detail', (c) {
        c.select(AppTab.peers);
        c.openPeer('cc330003');
      }),
    );
    testWidgets(
      'behaviour $lang',
      (t) => shot(t, 'behaviour', (c) => c.select(AppTab.behaviour)),
    );
    testWidgets(
      'settings $lang',
      (t) => shot(t, 'settings', (c) => c.select(AppTab.settings)),
    );
    testWidgets('onboarding safety $lang', (t) async {
      final c = controllerWith(sample(), lang: lang);
      await pumpApp(t, c, onboarded: false);
      for (var i = 0; i < 2; i++) {
        await t.tap(find.byKey(const Key('ob-next')));
        await t.pump(const Duration(milliseconds: 50));
      }
      await expectLater(
        find.byType(MaterialApp),
        matchesGoldenFile('goldens/onboarding-$lang.png'),
      );
    });
    testWidgets(
      'carplay $lang',
      (t) => shot(
        t,
        'carplay',
        (_) {},
        size: const Size(960, 480),
        speed: 48 / 3.6,
      ),
    );
  }
}
