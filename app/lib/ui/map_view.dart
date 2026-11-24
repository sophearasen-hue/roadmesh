/// The Drive map (AUDIT P-01 to P-07): me at the centre, north up, the radio range,
/// every peer as a heading arrow coloured by risk band, speed labels in Nearby view,
/// mesh links coloured by hop count, and the alert target ringed.
library;

import 'dart:math' as math;

import 'package:flutter/material.dart';

import '../radio.dart';
import 'style.dart';

/// Radio range drawn around me (m): Bluetooth LE in sight (SPEC §4, sim radio model).
const radioRangeM = 150.0;

/// Nearby view width (m), as in the prototype.
const nearbyWidthM = 520.0;

/// Screen geometry shared by the painter and the tap handler.
class MapGeometry {
  /// Geometry for `size`; Network view fits every peer.
  MapGeometry(this.size, List<Peer> peers, {required bool network})
    : scale = network ? _fit(size, peers) : size.width / nearbyWidthM;

  /// Canvas size.
  final Size size;

  /// Pixels per metre.
  final double scale;

  static double _fit(Size s, List<Peer> peers) {
    var far = radioRangeM;
    for (final p in peers) {
      far = math.max(far, math.max(p.east.abs(), p.north.abs()));
    }
    return math.min(s.width, s.height) / (far * 2 * 1.12);
  }

  /// Screen point of a position relative to me (north up).
  Offset at(double east, double north) =>
      Offset(size.width / 2 + east * scale, size.height / 2 - north * scale);

  /// The peer under a tap, within 22 px.
  Peer? hit(Offset tap, List<Peer> peers) {
    Peer? best;
    var bd = 22.0;
    for (final p in peers) {
      final d = (at(p.east, p.north) - tap).distance;
      if (d < bd) {
        bd = d;
        best = p;
      }
    }
    return best;
  }
}

/// For each relayed peer, the peer it most likely came through: the nearest one with
/// one hop fewer (the radio frame carries the last relay's position, not its identity).
Map<String, Peer?> meshParents(List<Peer> peers) {
  final out = <String, Peer?>{};
  for (final p in peers) {
    if (p.hops <= 0) {
      out[p.id] = null; // heard directly: linked to me
      continue;
    }
    Peer? best;
    var bd = double.infinity;
    for (final q in peers) {
      if (q.hops != p.hops - 1) continue;
      final d = math.sqrt(
        math.pow(q.east - p.east, 2) + math.pow(q.north - p.north, 2),
      );
      if (d < bd) {
        bd = d;
        best = q;
      }
    }
    out[p.id] = best;
  }
  return out;
}

/// Paints the map.
class MapPainter extends CustomPainter {
  /// `pulse` 0–1 animates the alert ring.
  MapPainter({
    required this.update,
    required this.network,
    required this.selected,
    required this.pulse,
    required this.speedLabels,
  });

  /// What to draw.
  final RadioUpdate update;

  /// Network view.
  final bool network;

  /// Selected peer display key.
  final String? selected;

  /// Animation phase.
  final double pulse;

  /// Speed labels as text spans (built by the widget, localised).
  final bool speedLabels;

  @override
  void paint(Canvas canvas, Size size) {
    final g = MapGeometry(size, update.peers, network: network);
    canvas.drawRect(
      Offset.zero & size,
      Paint()..color = const Color(0xFF0C1219),
    );
    final me = g.at(0, 0);
    // radio range
    final r = radioRangeM * g.scale;
    canvas.drawCircle(me, r, Paint()..color = const Color(0x123B82F6));
    _dashedCircle(canvas, me, r, const Color(0x733B82F6));
    // mesh links
    final parents = meshParents(update.peers);
    for (final p in update.peers) {
      final parent = parents[p.id];
      final from = parent == null ? me : g.at(parent.east, parent.north);
      canvas.drawLine(
        from,
        g.at(p.east, p.north),
        Paint()
          ..color = C.hops[math.min(p.hops + 1, 6)].withValues(alpha: 0.55)
          ..strokeWidth = 1.4,
      );
    }
    // alert targets (most important alert first)
    String? target;
    var targetColor = C.warn;
    for (final a in update.alerts) {
      if (a.target != null) {
        target = a.target;
        targetColor = a.level == 'danger' ? C.risk : C.warn;
        break;
      }
    }
    // peers
    for (final p in update.peers) {
      final s = g.at(p.east, p.north);
      if (s.dx < -20 ||
          s.dy < -20 ||
          s.dx > size.width + 20 ||
          s.dy > size.height + 20) {
        continue;
      }
      final len = _len(p.vehicleType) * (network ? 0.8 : 1.35);
      _arrow(
        canvas,
        s,
        p.heading,
        len,
        bandColor(band(p.score)),
        const Color(0xFF0B1220),
      );
      if (p.id == target) {
        canvas.drawCircle(
          s,
          13 + pulse * 5,
          Paint()
            ..color = targetColor
            ..style = PaintingStyle.stroke
            ..strokeWidth = 2,
        );
      }
      if (p.id == selected) {
        canvas.drawCircle(
          s,
          12,
          Paint()
            ..color = Colors.white
            ..style = PaintingStyle.stroke
            ..strokeWidth = 1.5,
        );
      }
      if (speedLabels && !network) {
        final tp = TextPainter(
          text: TextSpan(
            text: '${kmh(p.speed)}',
            style: const TextStyle(
              fontFamily: fontFamily,
              fontSize: 10,
              fontWeight: FontWeight.w600,
              color: Color(0xD9E2E8F0),
            ),
          ),
          textDirection: TextDirection.ltr,
        )..layout();
        tp.paint(canvas, s + const Offset(9, -18));
      }
    }
    // me
    canvas.drawCircle(me, 16, Paint()..color = const Color(0x383B82F6));
    _arrow(canvas, me, update.own.heading, 10, C.blue, Colors.white);
  }

  static double _len(int type) => switch (type) {
    1 => 5.5,
    2 => 6.5,
    3 || 4 => 9.5,
    _ => 7.5,
  };

  static void _arrow(
    Canvas c,
    Offset o,
    double headingDeg,
    double len,
    Color fill,
    Color stroke,
  ) {
    final h = headingDeg * math.pi / 180;
    final hx = math.sin(h), hy = -math.cos(h); // compass → screen
    final px = -hy, py = hx, w = len * 0.62;
    final path = Path()
      ..moveTo(o.dx + hx * len, o.dy + hy * len)
      ..lineTo(o.dx - hx * len * .7 + px * w, o.dy - hy * len * .7 + py * w)
      ..lineTo(o.dx - hx * len * .35, o.dy - hy * len * .35)
      ..lineTo(o.dx - hx * len * .7 - px * w, o.dy - hy * len * .7 - py * w)
      ..close();
    c.drawPath(path, Paint()..color = fill);
    c.drawPath(
      path,
      Paint()
        ..color = stroke
        ..style = PaintingStyle.stroke
        ..strokeWidth = 1.5,
    );
  }

  static void _dashedCircle(Canvas c, Offset o, double r, Color color) {
    final p = Paint()
      ..color = color
      ..style = PaintingStyle.stroke
      ..strokeWidth = 1;
    final n = math.max(12, (2 * math.pi * r / 8).floor());
    for (var i = 0; i < n; i += 2) {
      c.drawArc(
        Rect.fromCircle(center: o, radius: r),
        i * 2 * math.pi / n,
        2 * math.pi / n,
        false,
        p,
      );
    }
  }

  @override
  bool shouldRepaint(MapPainter old) => true;
}
