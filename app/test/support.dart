// Test helpers: a fixed engine update, the bundled fonts, and a golden comparator with
// a small tolerance (anti-aliasing differs slightly between machines).
import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:roadmesh_app/main.dart';
import 'package:roadmesh_app/radio.dart';
import 'package:roadmesh_app/ui/controller.dart';

/// A peer map as the drive service sends it.
Map<String, Object?> peer(
  String id, {
  required double east,
  required double north,
  double speed = 10,
  double heading = 0,
  int type = 1,
  int hops = 0,
  int score = 100,
  Map<String, int> counts = const {},
}) => {
  'id': id,
  'east': east,
  'north': north,
  'distance': Offset(east, north).distance,
  'speed': speed,
  'accel': -0.4,
  'heading': heading,
  'vehicleType': type,
  'hops': hops,
  'accuracy': 4.0,
  'ageMs': 30 + 30 * hops,
  'score': score,
  'counts': counts,
};

/// A busy street: one peer per risk band, a 3-hop chain, events and a Danger alert.
Map<String, Object?> sampleMap({
  List<Map<String, Object?>>? alerts,
  double speed = 0,
  double? limit = 40 / 3.6,
}) => {
  'alerts':
      alerts ??
      [
        {
          'kind': 'CrossingConflict',
          'level': 'danger',
          'advice': 'BrakeFirmly',
          'advisedDecel': 3.2,
          'distance': 48.0,
          'ttc': 3.4,
          'hops': 0,
          'target': 'aa110001',
        },
      ],
  'peers': [
    peer(
      'aa110001',
      east: 40,
      north: -30,
      heading: 0,
      score: 92,
      counts: {'speeding': 0},
    ),
    peer(
      'bb220002',
      east: -60,
      north: 20,
      heading: 90,
      type: 0,
      speed: 12,
      score: 74,
      counts: {'hbrake': 2, 'speeding': 1},
    ),
    peer(
      'cc330003',
      east: 120,
      north: 90,
      heading: 270,
      type: 2,
      speed: 6,
      hops: 1,
      score: 55,
      counts: {'speeding': 3, 'hacc': 2},
    ),
    peer(
      'dd440004',
      east: 230,
      north: 150,
      heading: 180,
      type: 3,
      speed: 14,
      hops: 2,
      score: 88,
    ),
  ],
  'own': {'speed': speed, 'heading': 90.0, 'accuracy': 4.0, 'limit': limit},
  'trip': {
    'distance': 12400.0,
    'avgSpeed': 8.9,
    'maxSpeed': 16.4,
    'score': 81,
    'counts': {'speeding': 2, 'hacc': 1, 'hbrake': 1, 'overtake': 3},
  },
  'events': [
    {'age': 12.0, 'kind': 'hbrake', 'me': false, 'id': 'bb220002', 'kmh': 41.0},
    {'age': 35.0, 'kind': 'speeding', 'me': true, 'id': '', 'kmh': 52.0},
    {
      'age': 61.0,
      'kind': 'speeding',
      'me': false,
      'id': 'cc330003',
      'kmh': 58.0,
    },
    {'age': 95.0, 'kind': 'overtake', 'me': true, 'id': '', 'kmh': 44.0},
  ],
};

/// The sample as an update.
RadioUpdate sample({List<Map<String, Object?>>? alerts, double speed = 0}) =>
    RadioUpdate.fromMap(sampleMap(alerts: alerts, speed: speed));

/// A controller with `updates` pushed (history builds at 1 sample per 5 updates).
DriveController controllerWith(
  RadioUpdate u, {
  String lang = 'en',
  int repeat = 1,
}) {
  final c = DriveController()..locale = Locale(lang);
  for (var i = 0; i < repeat; i++) {
    c.push(u);
  }
  return c;
}

/// Pump the app with `c` on a phone-sized screen (360 × 760 by default).
Future<void> pumpApp(
  WidgetTester t,
  DriveController c, {
  Size size = const Size(360, 760),
  bool onboarded = true,
}) async {
  if (onboarded) c.onboarded = true;
  t.view.physicalSize = size * 2;
  t.view.devicePixelRatio = 2;
  addTearDown(t.view.reset);
  await t.pumpWidget(RoadMeshApp(controller: c));
  await t.pump(const Duration(milliseconds: 50));
}

/// Load the bundled Kantumruy Pro and Material Icons so goldens show real text.
Future<void> loadFonts() async {
  Future<void> load(String family, List<String> files) async {
    final loader = FontLoader(family);
    for (final f in files) {
      final file = File(f);
      if (!file.existsSync()) return;
      loader.addFont(
        Future.value(ByteData.sublistView(file.readAsBytesSync())),
      );
    }
    await loader.load();
  }

  await load('Kantumruy Pro', [
    for (final w in ['Regular', 'Medium', 'SemiBold', 'Bold'])
      'assets/fonts/KantumruyPro-$w.ttf',
  ]);
  final root = Platform.environment['FLUTTER_ROOT'];
  if (root != null) {
    await load('MaterialIcons', [
      '$root/bin/cache/artifacts/material_fonts/MaterialIcons-Regular.otf',
    ]);
  }
}

/// Goldens pass within 1 % of pixels (font hinting differs between machines).
class TolerantComparator extends LocalFileComparator {
  /// Compare against goldens next to `testFile`.
  TolerantComparator(super.testFile);

  @override
  Future<bool> compare(Uint8List imageBytes, Uri golden) async {
    final result = await GoldenFileComparator.compareLists(
      imageBytes,
      await getGoldenBytes(golden),
    );
    if (result.passed || result.diffPercent <= 0.01) {
      result.dispose();
      return true;
    }
    final error = await generateFailureOutput(result, golden, basedir);
    result.dispose();
    throw FlutterError(error);
  }
}

/// Use [TolerantComparator] for the test file `path` (relative to app/).
void useTolerantGoldens(String path) {
  goldenFileComparator = TolerantComparator(
    Uri.file('${Directory.current.path}/$path'),
  );
}

/// Silence the method channel used by the app (tests run without the drive service).
void fakeRadioChannel() {
  TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
      .setMockMethodCallHandler(
        const MethodChannel('roadmesh/radio'),
        (call) async => null,
      );
}
