import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:roadmesh_app/main.dart';

void main() {
  testWidgets('shows English strings from app_en.arb', (tester) async {
    await tester.pumpWidget(const RoadMeshApp(locale: Locale('en')));
    await tester.pumpAndSettle();
    expect(find.text('RoadMesh'), findsOneWidget);
    expect(find.text('Peer-to-peer road safety'), findsOneWidget);
  });

  testWidgets('shows Khmer strings from app_km.arb', (tester) async {
    await tester.pumpWidget(const RoadMeshApp(locale: Locale('km')));
    await tester.pumpAndSettle();
    expect(find.text('សុវត្ថិភាពផ្លូវពីទូរស័ព្ទទៅទូរស័ព្ទ'), findsOneWidget);
  });
}
