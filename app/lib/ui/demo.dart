/// Debug-only demo traffic (AUDIT P-11): fake vehicles around me so the screens can be
/// tried without other phones, like the prototype's simulation. Never used in release
/// builds (`main.dart` creates it only under `kDebugMode`).
///
/// It is NOT the safety engine. Its one scripted alert (the hazard demo) is canned demo
/// data to show the banner; every real alert comes from the Rust core.
library;

import 'dart:async';
import 'dart:math' as math;

import '../radio.dart';
import 'controller.dart';

class _V {
  _V(this.id, this.type, this.x, this.y, this.h, this.v, this.score);
  final String id;
  final int type;
  double x, y, h, v; // metres, compass degrees, m/s
  int score;
  int hops = 0;
}

/// A small simulated neighbourhood, updated 5 times a second.
class DemoTraffic implements DemoHooks {
  /// `seed` makes runs repeatable.
  DemoTraffic({int seed = 7}) : _rnd = math.Random(seed) {
    for (var i = 0; i < 28; i++) {
      final horizontal = _rnd.nextBool();
      final lane = (_rnd.nextInt(5) - 2) * 300.0;
      final along = _rnd.nextDouble() * 1600 - 800;
      final dir = _rnd.nextBool() ? 1.0 : -1.0;
      _vs.add(
        _V(
          _rnd.nextInt(0xFFFFFFFF).toRadixString(16).padLeft(8, '0'),
          [1, 1, 1, 0, 0, 2, 3][_rnd.nextInt(7)],
          horizontal ? along : lane,
          horizontal ? lane : along,
          horizontal ? (dir > 0 ? 90 : 270) : (dir > 0 ? 0 : 180),
          6 + _rnd.nextDouble() * 9,
          [100, 96, 91, 88, 80, 72, 60, 52][_rnd.nextInt(8)],
        ),
      );
    }
  }

  final math.Random _rnd;
  final _vs = <_V>[];
  final _events = <Map<String, Object?>>[];
  double _mx = 0,
      _mv = 11,
      _bias = 0,
      _brakeT = 0,
      _t = 0,
      _dist = 0,
      _vmax = 0;
  double _vsum = 0;
  int _n = 0, _score = 100;
  double? _hazardUntil;
  _V? _hazard;
  final _counts = {'speeding': 0, 'hacc': 0, 'hbrake': 0, 'overtake': 0};

  static const _limit = 40 / 3.6;
  static const _range = 150.0;

  /// The updates.
  Stream<RadioUpdate> updates() =>
      Stream.periodic(const Duration(milliseconds: 200), (_) => step(0.2));

  @override
  void slower() => _bias = math.max(-10, _bias - 2);

  @override
  void faster() => _bias = math.min(12, _bias + 2);

  @override
  void brake() => _brakeT = 1.2;

  @override
  void hazard() {
    // a motorbike from the right, timed to meet me at the next junction (x = 300k)
    final next = ((_mx / 300).floor() + 1) * 300.0;
    final t = (next - _mx) / math.max(_mv, 3);
    final m = _V('demo0001', 1, next, -t * 11, 0, 11, 70);
    _vs.add(m);
    _hazard = m;
    _hazardUntil = _t + math.min(t, 7);
  }

  /// Advance `dt` seconds and build an update.
  RadioUpdate step(double dt) {
    _t += dt;
    final want = (_limit + _bias).clamp(0.0, 30.0);
    final a = _brakeT > 0 ? -6.5 : ((want - _mv) / 1.5).clamp(-7.0, 2.0);
    if (_brakeT > 0) _brakeT -= dt;
    _mv = math.max(0, _mv + a * dt);
    _mx += _mv * dt;
    _dist += _mv * dt;
    _vmax = math.max(_vmax, _mv);
    _vsum += _mv;
    _n++;
    for (final v in _vs) {
      final h = v.h * math.pi / 180;
      v.x += math.sin(h) * v.v * dt;
      v.y += math.cos(h) * v.v * dt;
      if (v.x - _mx > 900) v.x -= 1800;
      if (v.x - _mx < -900) v.x += 1800;
      if (v.y > 900) v.y -= 1800;
      if (v.y < -900) v.y += 1800;
    }
    // mesh: breadth-first from me within radio range
    for (final v in _vs) {
      v.hops = -1;
    }
    var frontier = <(double, double)>[(_mx, 0)];
    for (var hop = 0; hop < 4 && frontier.isNotEmpty; hop++) {
      final next = <(double, double)>[];
      for (final v in _vs) {
        if (v.hops >= 0) continue;
        for (final (fx, fy) in frontier) {
          if (math.sqrt(math.pow(v.x - fx, 2) + math.pow(v.y - fy, 2)) <=
              _range) {
            v.hops = hop;
            next.add((v.x, v.y));
            break;
          }
        }
      }
      frontier = next;
    }
    // occasional behaviour events of vehicles in my mesh
    if (_rnd.nextDouble() < 0.02) {
      final heard = _vs.where((v) => v.hops >= 0).toList();
      if (heard.isNotEmpty) {
        final v = heard[_rnd.nextInt(heard.length)];
        final kind = [
          'speeding',
          'hacc',
          'hbrake',
          'overtake',
        ][_rnd.nextInt(4)];
        v.score = math.max(0, v.score - 5);
        _events.insert(0, {
          't': _t,
          'kind': kind,
          'me': false,
          'id': v.id,
          'kmh': v.v * 3.6,
        });
      }
    }
    if (_mv > _limit + 10 / 3.6 && _rnd.nextDouble() < 0.01) {
      _counts['speeding'] = _counts['speeding']! + 1;
      _score = math.max(0, _score - 6);
      _events.insert(0, {
        't': _t,
        'kind': 'speeding',
        'me': true,
        'id': '',
        'kmh': _mv * 3.6,
      });
    }
    if (_events.length > 40) _events.removeLast();
    final alerts = <Map<String, Object?>>[];
    final hz = _hazard;
    if (hz != null && _hazardUntil != null && _t < _hazardUntil!) {
      alerts.add({
        'kind': 'CrossingConflict',
        'level': 'danger',
        'advice': 'BrakeFirmly',
        'advisedDecel': 3.0,
        'distance': math.sqrt(math.pow(hz.x - _mx, 2) + math.pow(hz.y, 2)),
        'ttc': _hazardUntil! - _t,
        'hops': 0,
        'target': hz.id,
      });
    } else if (_mv > _limit + 5 / 3.6) {
      alerts.add({
        'kind': 'OwnSpeeding',
        'level': 'warning',
        'advice': 'EaseOff',
        'hops': 0,
      });
    }
    return RadioUpdate.fromMap({
      'alerts': alerts,
      'peers': [
        for (final v in _vs)
          if (v.hops >= 0)
            {
              'id': v.id,
              'east': v.x - _mx,
              'north': v.y,
              'distance': math.sqrt(math.pow(v.x - _mx, 2) + math.pow(v.y, 2)),
              'speed': v.v,
              'accel': 0.0,
              'heading': v.h,
              'vehicleType': v.type,
              'hops': v.hops,
              'accuracy': 4.0,
              'ageMs': 30 + 30 * v.hops,
              'score': v.score,
              'counts': const <String, int>{},
            },
      ],
      'own': {'speed': _mv, 'heading': 90.0, 'accuracy': 4.0, 'limit': _limit},
      'trip': {
        'distance': _dist,
        'avgSpeed': _n == 0 ? 0 : _vsum / _n,
        'maxSpeed': _vmax,
        'score': _score,
        'counts': _counts,
      },
      'events': [
        for (final e in _events) {...e, 'age': _t - (e['t']! as double)},
      ],
    });
  }
}
