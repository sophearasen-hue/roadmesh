/// App state for the screens: the latest engine update, short speed histories for the
/// graphs, and the user's view choices. It never decides alerts (CLAUDE.md safety rules).
library;

import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:flutter/widgets.dart';
import 'package:shared_preferences/shared_preferences.dart';

import '../radio.dart';

/// Screens other than Drive lock above this speed (AUDIT P-16).
const lockAboveKmh = 10.0;

/// Seconds of speed history kept for the graphs (2 minutes at 1 Hz).
const historyLen = 120;

/// Updates per second from the drive service.
const updatesPerSecond = 5;

/// Driver controls of the debug demo (prototype speed +/−, hard brake, hazard demo).
/// Exists only in debug builds (AUDIT P-11).
abstract interface class DemoHooks {
  /// My speed −.
  void slower();

  /// My speed +.
  void faster();

  /// Hard brake.
  void brake();

  /// Hazard demo.
  void hazard();
}

/// The four tabs.
enum AppTab { drive, peers, behaviour, settings }

/// Holds what the screens show.
class DriveController extends ChangeNotifier {
  /// `prefs` remembers the language and motorbike mode.
  DriveController({this.prefs, this.onDeleteData}) {
    final lang = prefs?.getString(_kLang);
    if (lang == 'en' || lang == 'km') locale = Locale(lang!);
    motorbikeMode = prefs?.getBool(_kMoto) ?? false;
    muted = prefs?.getBool(_kMuted) ?? false;
    onboarded = prefs?.getBool(_kOnboarded) ?? false;
    vehicleType = prefs?.getInt(_kVehicle) ?? 1;
  }

  /// The driver has seen the safety notice and chosen language and vehicle (SPEC §9).
  bool onboarded = false;

  /// Own vehicle type code (0 car, 1 motorbike, 2 tuk-tuk, 3 truck) for the safety message.
  int vehicleType = 1;

  /// Vehicle type, remembered.
  Future<void> setVehicleType(int code) async {
    vehicleType = code;
    notifyListeners();
    await prefs?.setInt(_kVehicle, code);
  }

  /// The driver accepted the safety notice (AUDIT L-07).
  Future<void> finishOnboarding() async {
    onboarded = true;
    notifyListeners();
    await prefs?.setBool(_kOnboarded, true);
  }

  /// Alert sounds muted (from Settings or the CarPlay Mute button).
  bool muted = false;

  /// Plays the alert tone for a level ("warning" or "danger"); set by the app to the
  /// native sound player. The engine chose the alert; this only sounds it.
  void Function(String level)? onAlertTone;

  /// Level of the alert last sounded, so a tone plays once per escalation.
  String _sounded = 'ok';

  static const _kLang = 'language';
  static const _kMoto = 'motorbikeMode';
  static const _kMuted = 'muted';
  static const _kOnboarded = 'onboarded';
  static const _kVehicle = 'vehicleType';

  /// Persistent settings (null in some tests).
  final SharedPreferences? prefs;

  /// Deletes the data held by the drive service ("Delete my data").
  final Future<void> Function()? onDeleteData;

  /// Latest update, null before the first one.
  RadioUpdate? last;

  /// Speed history (km/h, 1 per second) per peer display key, and mine.
  final Map<String, List<int>> peerHistory = {};

  /// My speed history (km/h, 1 per second).
  final List<int> ownHistory = [];

  /// Chosen language; null = the phone's.
  Locale? locale;

  /// Motorbike mode (P-28): alerts by sound and vibration.
  bool motorbikeMode = false;

  /// Network view instead of Nearby view (P-06).
  bool networkView = false;

  /// Selected tab.
  AppTab tab = AppTab.drive;

  /// Peer opened in the Peers tab (display key).
  String? selected;

  /// Behaviour timeline filter: "all", "me", "peers".
  String filter = 'all';

  /// Debug demo controls; always null in release builds.
  DemoHooks? get demo => kDebugMode ? _demo : null;
  set demo(DemoHooks? d) => _demo = kDebugMode ? d : null;
  DemoHooks? _demo;

  /// Debug builds: starts the demo traffic (Settings → Debug tools). Null in release.
  VoidCallback? get onStartDemo => kDebugMode ? _onStartDemo : null;
  set onStartDemo(VoidCallback? f) => _onStartDemo = kDebugMode ? f : null;
  VoidCallback? _onStartDemo;

  /// Debug only: relay hop limit shown in Settings (release uses SPEC §5 TTLs, P-25).
  int debugMaxHops = 4;

  StreamSubscription<RadioUpdate>? _sub;
  int _n = 0;

  /// Listen to `updates` (the drive service, or the debug demo).
  void attach(Stream<RadioUpdate> updates) {
    _sub?.cancel();
    _sub = updates.listen(push);
  }

  /// Apply one update.
  void push(RadioUpdate u) {
    last = u;
    _sound(u);
    if (_n++ % updatesPerSecond == 0) {
      _add(ownHistory, (u.own.speed * 3.6).round());
      final ids = <String>{};
      for (final p in u.peers) {
        ids.add(p.id);
        _add(peerHistory.putIfAbsent(p.id, () => []), (p.speed * 3.6).round());
      }
      // peer data leaves with the peer (it is gone from the engine's map)
      peerHistory.removeWhere((id, _) => !ids.contains(id));
    }
    if (locked && tab != AppTab.drive) tab = AppTab.drive;
    if (selected != null && !u.peers.any((p) => p.id == selected)) {
      selected = null;
    }
    notifyListeners();
  }

  void _sound(RadioUpdate u) {
    var level = 'ok';
    for (final a in u.alerts) {
      if (a.level == 'danger') {
        level = 'danger';
        break;
      }
      if (a.level == 'warning') level = 'warning';
    }
    final rank = {'ok': 0, 'warning': 1, 'danger': 2};
    if (rank[level]! > rank[_sounded]! && !muted) onAlertTone?.call(level);
    _sounded = level;
  }

  /// Mute or unmute alert sounds, remembered.
  Future<void> setMuted(bool on) async {
    muted = on;
    notifyListeners();
    await prefs?.setBool(_kMuted, on);
  }

  static void _add(List<int> h, int v) {
    h.add(v);
    if (h.length > historyLen) h.removeAt(0);
  }

  /// My speed (km/h).
  double get speedKmh => (last?.own.speed ?? 0) * 3.6;

  /// Screens other than Drive are locked (P-16).
  bool get locked => speedKmh > lockAboveKmh;

  /// Select a tab; returns false when it is locked while driving.
  bool select(AppTab t) {
    if (t != AppTab.drive && locked) return false;
    tab = t;
    if (t == AppTab.peers) selected = null;
    notifyListeners();
    return true;
  }

  /// Open a peer's detail (map tap or list tap; P-10).
  void openPeer(String id) {
    selected = id;
    if (!locked) tab = AppTab.peers;
    notifyListeners();
  }

  /// Back from a peer's detail.
  void closePeer() {
    selected = null;
    notifyListeners();
  }

  /// Toggle Nearby / Network view.
  void toggleView() {
    networkView = !networkView;
    notifyListeners();
  }

  /// Timeline filter.
  void setFilter(String f) {
    filter = f;
    notifyListeners();
  }

  /// Language, applied at once and remembered (P-23).
  Future<void> setLanguage(String code) async {
    locale = Locale(code);
    notifyListeners();
    await prefs?.setString(_kLang, code);
  }

  /// Motorbike mode, remembered.
  Future<void> setMotorbikeMode(bool on) async {
    motorbikeMode = on;
    notifyListeners();
    await prefs?.setBool(_kMoto, on);
  }

  /// Debug relay-hop setting.
  void setDebugMaxHops(int n) {
    if (!kDebugMode) return;
    debugMaxHops = n;
    notifyListeners();
  }

  /// "Delete my data" (P-27): the service's data and everything shown here.
  Future<void> deleteMyData() async {
    await onDeleteData?.call();
    peerHistory.clear();
    ownHistory.clear();
    selected = null;
    last = null;
    notifyListeners();
  }

  @override
  void dispose() {
    _sub?.cancel();
    super.dispose();
  }
}
