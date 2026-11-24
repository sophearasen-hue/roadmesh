/// Small building blocks shared by the screens (prototype CSS classes in comments).
library;

import 'dart:math' as math;

import 'package:flutter/material.dart';

import '../l10n/app_localizations.dart';
import 'style.dart';

/// `.kv` grid: label above value, two per row.
class KvGrid extends StatelessWidget {
  /// Pairs of label and value.
  const KvGrid(this.items, {super.key});

  /// (label, value).
  final List<(String, String)> items;

  @override
  Widget build(BuildContext context) => LayoutBuilder(
    builder: (context, c) {
      final w = (c.maxWidth - 8) / 2;
      return Wrap(
        spacing: 8,
        runSpacing: 8,
        children: [
          for (final (k, v) in items)
            Container(
              width: w,
              padding: const EdgeInsets.symmetric(horizontal: 10, vertical: 9),
              decoration: BoxDecoration(
                color: C.panel2,
                borderRadius: BorderRadius.circular(12),
              ),
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  Text(k, style: const TextStyle(fontSize: 11, color: C.muted)),
                  Text(
                    v,
                    style: const TextStyle(
                      fontSize: 15,
                      fontWeight: FontWeight.w700,
                    ),
                  ),
                ],
              ),
            ),
        ],
      );
    },
  );
}

/// `.chips`: pill buttons.
class Chips extends StatelessWidget {
  /// `selected` marks the chip that is on.
  const Chips(this.labels, {super.key, this.selected, this.onTap, this.keys});

  /// Chip texts.
  final List<String> labels;

  /// Index of the chip that is on.
  final int? selected;

  /// Tap handler (index).
  final ValueChanged<int>? onTap;

  /// Widget keys for tests.
  final List<Key>? keys;

  @override
  Widget build(BuildContext context) => Wrap(
    spacing: 6,
    runSpacing: 6,
    children: [
      for (var i = 0; i < labels.length; i++)
        InkWell(
          key: keys?[i],
          borderRadius: BorderRadius.circular(999),
          onTap: onTap == null ? null : () => onTap!(i),
          child: Container(
            padding: const EdgeInsets.symmetric(horizontal: 11, vertical: 5),
            decoration: BoxDecoration(
              color: C.panel2,
              borderRadius: BorderRadius.circular(999),
              border: Border.all(color: i == selected ? C.blue : C.line),
            ),
            child: Text(
              labels[i],
              style: TextStyle(
                fontSize: 12,
                color: i == selected ? C.text : C.muted,
              ),
            ),
          ),
        ),
    ],
  );
}

/// Risk badge `.badge`.
class RiskBadge extends StatelessWidget {
  /// Badge text and band.
  const RiskBadge(this.text, this.band, {super.key});

  /// Text.
  final String text;

  /// Band.
  final Band band;

  @override
  Widget build(BuildContext context) {
    final (bg, fg) = switch (band) {
      Band.safe => (const Color(0x2622C55E), const Color(0xFF86EFAC)),
      Band.caution => (const Color(0x26F59E0B), const Color(0xFFFCD34D)),
      Band.risky => (const Color(0x2EEF4444), const Color(0xFFFCA5A5)),
    };
    return Container(
      margin: const EdgeInsets.only(top: 3),
      padding: const EdgeInsets.symmetric(horizontal: 7, vertical: 2),
      decoration: BoxDecoration(
        color: bg,
        borderRadius: BorderRadius.circular(999),
      ),
      child: Text(
        text,
        style: TextStyle(
          fontSize: 10.5,
          fontWeight: FontWeight.w600,
          color: fg,
        ),
      ),
    );
  }
}

/// Vehicle type tile `.ico`.
class TypeTile extends StatelessWidget {
  /// Type code and size.
  const TypeTile(this.type, {super.key, this.size = 34});

  /// Type code.
  final int type;

  /// Side (px).
  final double size;

  @override
  Widget build(BuildContext context) => Container(
    width: size,
    height: size,
    alignment: Alignment.center,
    decoration: BoxDecoration(
      color: C.type(type),
      borderRadius: BorderRadius.circular(10),
    ),
    child: Text(
      typeTag(type),
      style: const TextStyle(
        fontSize: 10,
        fontWeight: FontWeight.w700,
        color: Color(0xFF0B1220),
      ),
    ),
  );
}

/// Section heading `h3`.
class SectionTitle extends StatelessWidget {
  /// Heading text.
  const SectionTitle(this.text, {super.key});

  /// Text.
  final String text;

  @override
  Widget build(BuildContext context) => Padding(
    padding: const EdgeInsets.only(top: 16, bottom: 8),
    child: Text(
      text,
      style: const TextStyle(
        fontSize: 13,
        fontWeight: FontWeight.w600,
        color: C.muted,
      ),
    ),
  );
}

/// 2-minute speed graph with a dashed limit line (`.spark`, P-15, P-20).
class SpeedGraph extends StatelessWidget {
  /// km/h samples, 1 per second; `limitKmh` draws the limit line.
  const SpeedGraph(this.samples, {super.key, this.limitKmh});

  /// Samples.
  final List<int> samples;

  /// Speed limit (km/h), if known.
  final int? limitKmh;

  @override
  Widget build(BuildContext context) {
    final l = AppLocalizations.of(context);
    return Container(
      height: 90,
      decoration: BoxDecoration(
        color: C.panel2,
        borderRadius: BorderRadius.circular(12),
      ),
      child: CustomPaint(
        size: Size.infinite,
        painter: _SparkPainter(samples, limitKmh, l.kmh),
      ),
    );
  }
}

class _SparkPainter extends CustomPainter {
  _SparkPainter(this.arr, this.limit, this.unit);
  final List<int> arr;
  final int? limit;
  final String unit;

  @override
  void paint(Canvas c, Size s) {
    final mx = math
        .max((limit ?? 0) + 20, arr.fold<int>(10, (a, b) => math.max(a, b)))
        .toDouble();
    double y(num v) => s.height - 8 - (v / mx) * (s.height - 18);
    if (limit != null) {
      final ly = y(limit!);
      final p = Paint()
        ..color = const Color(0x99EF4444)
        ..strokeWidth = 1;
      for (double x = 0; x < s.width; x += 8) {
        c.drawLine(Offset(x, ly), Offset(math.min(x + 4, s.width), ly), p);
      }
      final tp = TextPainter(
        text: TextSpan(
          text: '$limit $unit',
          style: const TextStyle(
            fontFamily: fontFamily,
            fontSize: 10,
            color: Color(0xCCEF4444),
          ),
        ),
        textDirection: TextDirection.ltr,
      )..layout();
      tp.paint(c, Offset(6, ly - 4 - tp.height));
    }
    if (arr.length < 2) return;
    final path = Path();
    for (var i = 0; i < arr.length; i++) {
      final x = s.width - (arr.length - 1 - i) * (s.width / 119);
      final pt = Offset(x, y(arr[i]));
      i == 0 ? path.moveTo(pt.dx, pt.dy) : path.lineTo(pt.dx, pt.dy);
    }
    c.drawPath(
      path,
      Paint()
        ..color = const Color(0xFF60A5FA)
        ..strokeWidth = 2
        ..style = PaintingStyle.stroke,
    );
  }

  @override
  bool shouldRepaint(_SparkPainter o) => true;
}

/// Trip score ring (P-17).
class ScoreRing extends StatelessWidget {
  /// Score 0–100.
  const ScoreRing(this.score, {super.key});

  /// Score.
  final int score;

  @override
  Widget build(BuildContext context) => SizedBox(
    width: 76,
    height: 76,
    child: CustomPaint(
      painter: _RingPainter(score),
      child: Center(
        child: Text(
          '$score',
          style: const TextStyle(fontSize: 19, fontWeight: FontWeight.w700),
        ),
      ),
    ),
  );
}

class _RingPainter extends CustomPainter {
  _RingPainter(this.score);
  final int score;

  @override
  void paint(Canvas c, Size s) {
    final rect = Rect.fromCircle(center: s.center(Offset.zero), radius: 30);
    c.drawArc(
      rect,
      0,
      2 * math.pi,
      false,
      Paint()
        ..color = C.line
        ..style = PaintingStyle.stroke
        ..strokeWidth = 7,
    );
    c.drawArc(
      rect,
      -math.pi / 2,
      2 * math.pi * score.clamp(0, 100) / 100,
      false,
      Paint()
        ..color = bandColor(band(score))
        ..style = PaintingStyle.stroke
        ..strokeWidth = 7
        ..strokeCap = StrokeCap.round,
    );
  }

  @override
  bool shouldRepaint(_RingPainter o) => o.score != score;
}

/// One timeline row `.ev`.
class EventRow extends StatelessWidget {
  /// `who` is already localised.
  const EventRow({
    super.key,
    required this.ago,
    required this.kind,
    required this.who,
  });

  /// "12s ago".
  final String ago;

  /// Event key.
  final String kind;

  /// "You · 72 km/h".
  final String who;

  @override
  Widget build(BuildContext context) {
    final l = AppLocalizations.of(context);
    final (bg, fg) = eventColors(kind);
    return Container(
      padding: const EdgeInsets.symmetric(vertical: 8, horizontal: 2),
      decoration: const BoxDecoration(
        border: Border(bottom: BorderSide(color: C.line)),
      ),
      child: Row(
        children: [
          SizedBox(
            width: 56,
            child: Text(
              ago,
              style: const TextStyle(fontSize: 12, color: C.muted),
            ),
          ),
          Container(
            padding: const EdgeInsets.symmetric(horizontal: 7, vertical: 2),
            decoration: BoxDecoration(
              color: bg,
              borderRadius: BorderRadius.circular(6),
            ),
            child: Text(
              eventName(l, kind),
              style: TextStyle(
                fontSize: 10.5,
                fontWeight: FontWeight.w600,
                color: fg,
              ),
            ),
          ),
          const SizedBox(width: 10),
          Expanded(
            child: Text(
              who,
              style: const TextStyle(fontSize: 12.5),
              overflow: TextOverflow.ellipsis,
            ),
          ),
        ],
      ),
    );
  }
}

/// Empty-state text `.empty`.
class EmptyText extends StatelessWidget {
  /// Text.
  const EmptyText(this.text, {super.key});

  /// Text.
  final String text;

  @override
  Widget build(BuildContext context) => Padding(
    padding: const EdgeInsets.symmetric(vertical: 20, horizontal: 4),
    child: Text(
      text,
      style: const TextStyle(fontSize: 13, color: C.muted, height: 1.6),
    ),
  );
}
