import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:shared_preferences/shared_preferences.dart';

import 'l10n/app_localizations.dart';
import 'radio.dart';
import 'ui/carplay.dart';
import 'ui/controller.dart';
import 'ui/demo.dart';
import 'ui/home.dart';
import 'ui/style.dart';

Future<void> main() async {
  WidgetsFlutterBinding.ensureInitialized();
  final prefs = await SharedPreferences.getInstance();
  final c = DriveController(
    prefs: prefs,
    onDeleteData: RoadMeshRadio.deleteMyData,
  );
  runApp(RoadMeshApp(controller: c, live: true));
}

/// Root widget. All user-facing text comes from `lib/l10n/app_*.arb`.
class RoadMeshApp extends StatefulWidget {
  /// `live` connects to the drive service; tests pass a controller and feed it.
  const RoadMeshApp({super.key, required this.controller, this.live = false});

  /// State.
  final DriveController controller;

  /// Listen to the native drive service.
  final bool live;

  @override
  State<RoadMeshApp> createState() => _RoadMeshAppState();
}

class _RoadMeshAppState extends State<RoadMeshApp> {
  bool _driving = false;
  DemoTraffic? _demo;
  CarPlayLink? _carplay;

  DriveController get c => widget.controller;

  @override
  void initState() {
    super.initState();
    if (widget.live) {
      c.attach(RoadMeshRadio.updates);
      c.onAlertTone = RoadMeshRadio.alertTone;
      _carplay = CarPlayLink(onMute: c.setMuted);
    }
  }

  Future<void> _toggleDrive(BuildContext context) async {
    final l = AppLocalizations.of(context);
    if (_driving) {
      await RoadMeshRadio.stop();
      setState(() => _driving = false);
      return;
    }
    if (!await RoadMeshRadio.requestPermissions()) {
      // debug builds fall back to demo traffic so the screens can be tried (P-11)
      if (kDebugMode) _startDemo();
      return;
    }
    await RoadMeshRadio.start(
      notificationTitle: l.notifTitle,
      notificationText: l.notifText,
      vehicleType: c.motorbikeMode ? 1 : 0,
    );
    setState(() => _driving = true);
  }

  void _startDemo() {
    if (!kDebugMode) return;
    final d = DemoTraffic();
    _demo = d;
    c.demo = d;
    c.attach(d.updates());
    setState(() => _driving = true);
  }

  @override
  Widget build(BuildContext context) => ListenableBuilder(
    listenable: c,
    builder: (context, _) => MaterialApp(
      debugShowCheckedModeBanner: false,
      onGenerateTitle: (context) => AppLocalizations.of(context).appTitle,
      theme: roadMeshTheme(),
      locale: c.locale,
      localizationsDelegates: AppLocalizations.localizationsDelegates,
      supportedLocales: AppLocalizations.supportedLocales,
      home: Builder(
        builder: (context) {
          final u = c.last;
          if (_carplay != null && u != null) {
            _carplay!.show(
              carPlayModel(AppLocalizations.of(context), u, muted: c.muted),
            );
          }
          return HomeScreen(
            c: c,
            driving: _driving,
            onDrive: widget.live && _demo == null
                ? () => _toggleDrive(context)
                : null,
          );
        },
      ),
    ),
  );
}
