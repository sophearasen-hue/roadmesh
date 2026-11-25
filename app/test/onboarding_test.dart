// First run (SPEC §9 onboarding, AUDIT L-07): language, vehicle type, safety notice.
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:roadmesh_app/ui/controller.dart';
import 'package:shared_preferences/shared_preferences.dart';

import 'support.dart';

void main() {
  setUp(fakeRadioChannel);

  testWidgets('first run: language, vehicle, safety notice; remembered', (
    t,
  ) async {
    SharedPreferences.setMockInitialValues({});
    final prefs = await SharedPreferences.getInstance();
    final c = DriveController(prefs: prefs)..locale = const Locale('en');
    await pumpApp(t, c, onboarded: false);
    expect(find.text('Choose your language'), findsOneWidget);
    expect(
      find.byKey(const Key('tab-drive')),
      findsNothing,
    ); // no app before the notice
    await t.tap(find.byKey(const Key('ob-km')));
    await t.pump(const Duration(milliseconds: 50));
    expect(find.text('ជ្រើសរើសភាសារបស់អ្នក'), findsOneWidget);
    await t.tap(find.byKey(const Key('ob-next')));
    await t.pump(const Duration(milliseconds: 50));
    await t.tap(find.byKey(const Key('ob-vehicle-2'))); // tuk-tuk
    await t.tap(find.byKey(const Key('ob-next')));
    await t.pump(const Duration(milliseconds: 50));
    expect(find.byKey(const Key('safetyText')), findsOneWidget);
    await t.tap(find.byKey(const Key('ob-accept')));
    await t.pump(const Duration(milliseconds: 100));
    expect(find.byKey(const Key('tab-drive')), findsOneWidget);
    final again = DriveController(prefs: prefs);
    expect(
      (again.onboarded, again.vehicleType, again.locale),
      (true, 2, const Locale('km')),
    );
  });

  testWidgets('the safety notice says the app never brakes or steers', (
    t,
  ) async {
    final c = DriveController()..locale = const Locale('en');
    await pumpApp(t, c, onboarded: false);
    for (var i = 0; i < 2; i++) {
      await t.tap(find.byKey(const Key('ob-next')));
      await t.pump(const Duration(milliseconds: 50));
    }
    expect(find.textContaining('never brakes or steers'), findsOneWidget);
    expect(find.textContaining('not a guarantee'), findsOneWidget);
  });
}
