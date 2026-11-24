/// Drive tab (AUDIT P-01 to P-11) and its CarPlay-sized layout with a side panel.
library;

import 'package:flutter/material.dart';

import '../l10n/app_localizations.dart';
import '../radio.dart';
import 'controller.dart';
import 'map_view.dart';
import 'style.dart';

/// Map, alert banner, legend, view toggle and HUD.
class DriveScreen extends StatefulWidget {
  /// `carplay` shows the side panel instead of the HUD.
  const DriveScreen({super.key, required this.c, this.carplay = false});

  /// State.
  final DriveController c;

  /// CarPlay-sized layout.
  final bool carplay;

  @override
  State<DriveScreen> createState() => _DriveScreenState();
}

class _DriveScreenState extends State<DriveScreen>
    with SingleTickerProviderStateMixin {
  late final AnimationController _pulse = AnimationController(
    vsync: this,
    duration: const Duration(milliseconds: 1000),
  );

  @override
  void initState() {
    super.initState();
    _pulse.repeat(reverse: true);
  }

  @override
  void dispose() {
    _pulse.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final c = widget.c;
    final u = c.last ?? emptyUpdate;
    final map = Stack(
      children: [
        Positioned.fill(
          child: LayoutBuilder(
            builder: (context, box) => GestureDetector(
              key: const Key('map'),
              onTapUp: (d) {
                final g = MapGeometry(
                  box.biggest,
                  u.peers,
                  network: c.networkView,
                );
                final p = g.hit(d.localPosition, u.peers);
                if (p != null) c.openPeer(p.id);
              },
              child: AnimatedBuilder(
                animation: _pulse,
                builder: (context, _) => CustomPaint(
                  size: box.biggest,
                  painter: MapPainter(
                    update: u,
                    network: c.networkView,
                    selected: c.selected,
                    pulse: _pulse.value,
                    speedLabels: true,
                  ),
                ),
              ),
            ),
          ),
        ),
        Positioned(
          left: 10,
          right: 10,
          top: 10,
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              AlertBanner(update: u),
              const SizedBox(height: 6),
              Row(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  Expanded(child: Legend(showUnconnected: c.demo != null)),
                  if (c.demo != null) ...[
                    const SizedBox(width: 6),
                    _MapButton(
                      key: const Key('demoBtn'),
                      text: AppLocalizations.of(context).demo,
                      onTap: c.demo!.hazard,
                    ),
                  ],
                  const SizedBox(width: 6),
                  _MapButton(
                    key: const Key('viewToggle'),
                    text: c.networkView
                        ? AppLocalizations.of(context).nearbyView
                        : AppLocalizations.of(context).networkView,
                    onTap: c.toggleView,
                  ),
                ],
              ),
            ],
          ),
        ),
        if (!widget.carplay)
          Positioned(left: 10, right: 10, bottom: 10, child: Hud(c: c)),
      ],
    );
    if (!widget.carplay) return map;
    return Row(
      children: [
        Expanded(child: map),
        SizedBox(width: 300, child: CarPlaySide(c: c)),
      ],
    );
  }
}

/// An update with nothing in it (before the first one arrives).
final emptyUpdate = RadioUpdate.fromMap(const {});

class _MapButton extends StatelessWidget {
  const _MapButton({super.key, required this.text, required this.onTap});
  final String text;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) => InkWell(
    onTap: onTap,
    borderRadius: BorderRadius.circular(10),
    child: Container(
      padding: const EdgeInsets.symmetric(horizontal: 10, vertical: 6),
      decoration: BoxDecoration(
        color: const Color(0xEB111821),
        borderRadius: BorderRadius.circular(10),
        border: Border.all(color: C.line),
      ),
      child: Text(text, style: const TextStyle(fontSize: 12)),
    ),
  );
}

/// The alert banner (P-08): green all clear, amber Warning, red pulsing Danger.
/// Caution alerts only highlight the map.
class AlertBanner extends StatefulWidget {
  /// The update to show.
  const AlertBanner({super.key, required this.update});

  /// Update.
  final RadioUpdate update;

  @override
  State<AlertBanner> createState() => _AlertBannerState();
}

class _AlertBannerState extends State<AlertBanner>
    with SingleTickerProviderStateMixin {
  late final AnimationController _a = AnimationController(
    vsync: this,
    duration: const Duration(milliseconds: 500),
  );

  @override
  void dispose() {
    _a.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final l = AppLocalizations.of(context);
    Alert? top;
    for (final a in widget.update.alerts) {
      if (a.level == 'warning' || a.level == 'danger') {
        top = a;
        break;
      }
    }
    final danger = top?.level == 'danger';
    if (danger) {
      if (!_a.isAnimating) _a.repeat(reverse: true);
    } else {
      _a.stop();
    }
    final (bg, border, dot) = top == null
        ? (const Color(0xEB14202C), C.line, C.safe)
        : danger
        ? (const Color(0xF07F1414), C.risk, Colors.white)
        : (const Color(0xEB784808), const Color(0xFFB7791F), C.warn);
    final text = top == null ? l.noAlert : alertText(l, top, widget.update);
    return AnimatedBuilder(
      animation: _a,
      builder: (context, child) => Container(
        key: Key('alert-${top?.level ?? 'ok'}'),
        padding: const EdgeInsets.symmetric(horizontal: 14, vertical: 11),
        decoration: BoxDecoration(
          color: bg,
          borderRadius: BorderRadius.circular(14),
          border: Border.all(color: border),
          boxShadow: danger
              ? [
                  BoxShadow(
                    color: const Color(0x59EF4444),
                    spreadRadius: 4 * _a.value,
                  ),
                ]
              : null,
        ),
        child: child,
      ),
      child: Row(
        children: [
          Container(
            width: 10,
            height: 10,
            decoration: BoxDecoration(color: dot, shape: BoxShape.circle),
          ),
          const SizedBox(width: 10),
          Expanded(
            child: Text(
              text,
              key: const Key('alertText'),
              style: const TextStyle(
                fontSize: 14,
                fontWeight: FontWeight.w500,
                height: 1.45,
              ),
            ),
          ),
        ],
      ),
    );
  }
}

/// Map legend (P-07).
class Legend extends StatelessWidget {
  /// `showUnconnected`: demo only (the phone cannot see vehicles outside the mesh).
  const Legend({super.key, this.showUnconnected = false});

  /// Show "Not in mesh".
  final bool showUnconnected;

  @override
  Widget build(BuildContext context) {
    final l = AppLocalizations.of(context);
    final items = [
      (C.blue, l.legendMe),
      (C.safe, l.riskSafe),
      (C.warn, l.riskCaution),
      (C.risk, l.riskRisky),
      if (showUnconnected) (C.unconnected, l.legendUnc),
    ];
    return Container(
      padding: const EdgeInsets.symmetric(horizontal: 9, vertical: 6),
      decoration: BoxDecoration(
        color: const Color(0xD9111821),
        borderRadius: BorderRadius.circular(10),
        border: Border.all(color: C.line),
      ),
      child: Wrap(
        spacing: 10,
        runSpacing: 2,
        children: [
          for (final (col, t) in items)
            Row(
              mainAxisSize: MainAxisSize.min,
              children: [
                Container(
                  width: 8,
                  height: 8,
                  decoration: BoxDecoration(color: col, shape: BoxShape.circle),
                ),
                const SizedBox(width: 4),
                Text(t, style: const TextStyle(fontSize: 10.5, color: C.muted)),
              ],
            ),
        ],
      ),
    );
  }
}

/// Speed-limit sign; pulses when I am more than 5 km/h over (P-09).
class LimitSign extends StatefulWidget {
  /// `limit` km/h; `over` pulses.
  const LimitSign({super.key, required this.limit, required this.over});

  /// km/h.
  final int limit;

  /// Over the limit.
  final bool over;

  @override
  State<LimitSign> createState() => _LimitSignState();
}

class _LimitSignState extends State<LimitSign>
    with SingleTickerProviderStateMixin {
  late final AnimationController _a = AnimationController(
    vsync: this,
    duration: const Duration(milliseconds: 500),
  );

  @override
  void dispose() {
    _a.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    if (widget.over) {
      if (!_a.isAnimating) _a.repeat(reverse: true);
    } else {
      _a.stop();
    }
    return AnimatedBuilder(
      animation: _a,
      builder: (context, _) => Container(
        key: Key(widget.over ? 'limit-over' : 'limit'),
        width: 42,
        height: 42,
        alignment: Alignment.center,
        decoration: BoxDecoration(
          color: Colors.white,
          shape: BoxShape.circle,
          border: Border.all(color: const Color(0xFFDC2626), width: 4),
          boxShadow: widget.over
              ? [
                  BoxShadow(
                    color: const Color(0x59EF4444),
                    spreadRadius: 4 * _a.value,
                  ),
                ]
              : null,
        ),
        child: Text(
          '${widget.limit}',
          style: const TextStyle(
            fontWeight: FontWeight.w700,
            fontSize: 15,
            color: Color(0xFF111111),
          ),
        ),
      ),
    );
  }
}

/// Mesh reach: the farthest vehicle in my mesh (m).
double meshReach(RadioUpdate u) =>
    u.peers.fold(0.0, (m, p) => p.distance > m ? p.distance : m);

/// Speed over the limit by more than 5 km/h.
bool overLimit(RadioUpdate u) =>
    u.own.limit != null && kmh(u.own.speed) > kmh(u.own.limit!) + 5;

/// HUD (P-09): my speed, limit sign, peers connected, mesh reach, trip score, road.
class Hud extends StatelessWidget {
  /// State.
  const Hud({super.key, required this.c});

  /// State.
  final DriveController c;

  @override
  Widget build(BuildContext context) {
    final l = AppLocalizations.of(context);
    final u = c.last ?? emptyUpdate;
    final demo = c.demo;
    Widget stat(String v, String k, Key key) => Expanded(
      child: Container(
        margin: const EdgeInsets.symmetric(horizontal: 2),
        padding: const EdgeInsets.symmetric(horizontal: 3, vertical: 6),
        decoration: BoxDecoration(
          color: C.panel2,
          borderRadius: BorderRadius.circular(10),
        ),
        child: Column(
          children: [
            FittedBox(
              fit: BoxFit.scaleDown,
              child: Text(
                v,
                key: key,
                maxLines: 1,
                style: const TextStyle(
                  fontSize: 15,
                  fontWeight: FontWeight.w700,
                ),
              ),
            ),
            FittedBox(
              fit: BoxFit.scaleDown,
              child: Text(
                k,
                maxLines: 1,
                style: const TextStyle(fontSize: 10.5, color: C.muted),
              ),
            ),
          ],
        ),
      ),
    );
    return Container(
      padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 10),
      decoration: BoxDecoration(
        color: const Color(0xF0111821),
        borderRadius: BorderRadius.circular(16),
        border: Border.all(color: C.line),
      ),
      child: Column(
        children: [
          Row(
            children: [
              if (u.own.limit != null) ...[
                LimitSign(limit: kmh(u.own.limit!), over: overLimit(u)),
                const SizedBox(width: 10),
              ],
              Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  Text(
                    '${kmh(u.own.speed)}',
                    key: const Key('mySpeed'),
                    style: const TextStyle(
                      fontSize: 34,
                      fontWeight: FontWeight.w700,
                      height: 1,
                    ),
                  ),
                  Text(
                    l.kmh,
                    style: const TextStyle(fontSize: 11, color: C.muted),
                  ),
                ],
              ),
              const SizedBox(width: 8),
              stat('${u.peers.length}', l.connected, const Key('peersCount')),
              stat(fmtDist(l, meshReach(u)), l.reach, const Key('reach')),
              stat('${u.trip.score}', l.score, const Key('score')),
            ],
          ),
          const SizedBox(height: 8),
          Row(
            children: [
              if (demo != null) ...[
                Text(
                  l.mySpeed,
                  style: const TextStyle(fontSize: 12, color: C.muted),
                ),
                const SizedBox(width: 6),
                _CtrlButton(
                  key: const Key('slower'),
                  text: '−',
                  onTap: demo.slower,
                ),
                _CtrlButton(
                  key: const Key('faster'),
                  text: '+',
                  onTap: demo.faster,
                ),
                _CtrlButton(
                  key: const Key('brake'),
                  text: l.brake,
                  onTap: demo.brake,
                  danger: true,
                ),
              ],
              const Spacer(),
              Flexible(
                child: Text(
                  l.unknownRoad,
                  key: const Key('roadName'),
                  textAlign: TextAlign.right,
                  style: const TextStyle(fontSize: 12),
                  overflow: TextOverflow.ellipsis,
                ),
              ),
            ],
          ),
        ],
      ),
    );
  }
}

class _CtrlButton extends StatelessWidget {
  const _CtrlButton({
    super.key,
    required this.text,
    required this.onTap,
    this.danger = false,
  });
  final String text;
  final VoidCallback onTap;
  final bool danger;

  @override
  Widget build(BuildContext context) => Padding(
    padding: const EdgeInsets.only(right: 6),
    child: InkWell(
      onTap: onTap,
      borderRadius: BorderRadius.circular(9),
      child: Container(
        padding: const EdgeInsets.symmetric(horizontal: 10, vertical: 5),
        decoration: BoxDecoration(
          color: C.panel2,
          borderRadius: BorderRadius.circular(9),
          border: Border.all(color: danger ? const Color(0xFF7F1D1D) : C.line),
        ),
        child: Text(
          text,
          style: TextStyle(
            fontSize: danger ? 12 : 14,
            fontWeight: FontWeight.w600,
            color: danger ? const Color(0xFFFCA5A5) : C.text,
          ),
        ),
      ),
    ),
  );
}

/// The CarPlay side panel (prototype `.side`): speed, limit, score, mesh line and the
/// five nearest vehicles.
class CarPlaySide extends StatelessWidget {
  /// State.
  const CarPlaySide({super.key, required this.c});

  /// State.
  final DriveController c;

  @override
  Widget build(BuildContext context) {
    final l = AppLocalizations.of(context);
    final u = c.last ?? emptyUpdate;
    final near = [...u.peers]..sort((a, b) => a.distance.compareTo(b.distance));
    BoxDecoration card() =>
        BoxDecoration(color: C.panel2, borderRadius: BorderRadius.circular(12));
    return Container(
      decoration: const BoxDecoration(
        border: Border(left: BorderSide(color: C.line)),
      ),
      padding: const EdgeInsets.all(12),
      child: ListView(
        children: [
          Container(
            padding: const EdgeInsets.all(10),
            decoration: card(),
            child: Row(
              children: [
                if (u.own.limit != null) ...[
                  LimitSign(limit: kmh(u.own.limit!), over: overLimit(u)),
                  const SizedBox(width: 12),
                ],
                Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    Text(
                      '${kmh(u.own.speed)}',
                      style: const TextStyle(
                        fontSize: 40,
                        fontWeight: FontWeight.w700,
                        height: 1,
                      ),
                    ),
                    Text(
                      l.kmh,
                      style: const TextStyle(fontSize: 11, color: C.muted),
                    ),
                  ],
                ),
                const Spacer(),
                Column(
                  crossAxisAlignment: CrossAxisAlignment.end,
                  children: [
                    Text(
                      l.score,
                      style: const TextStyle(fontSize: 12, color: C.muted),
                    ),
                    Text(
                      '${u.trip.score}',
                      style: const TextStyle(
                        fontSize: 20,
                        fontWeight: FontWeight.w700,
                      ),
                    ),
                  ],
                ),
              ],
            ),
          ),
          const SizedBox(height: 10),
          Container(
            padding: const EdgeInsets.all(10),
            decoration: card(),
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Text(
                  l.unknownRoad,
                  style: const TextStyle(fontSize: 12.5, color: C.muted),
                ),
                const SizedBox(height: 4),
                Text(
                  l.meshLine(u.peers.length, fmtDist(l, meshReach(u))),
                  style: const TextStyle(fontSize: 12.5, color: C.muted),
                ),
              ],
            ),
          ),
          const SizedBox(height: 10),
          Container(
            padding: const EdgeInsets.all(10),
            decoration: card(),
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.stretch,
              children: [
                Text(
                  l.nearest,
                  style: const TextStyle(fontSize: 12, color: C.muted),
                ),
                const SizedBox(height: 4),
                if (near.isEmpty) const Text('—'),
                for (final p in near.take(5))
                  Padding(
                    padding: const EdgeInsets.symmetric(vertical: 4),
                    child: Row(
                      children: [
                        Text(
                          '● ',
                          style: TextStyle(color: bandColor(band(p.score))),
                        ),
                        Expanded(
                          child: Text(
                            typeName(l, p.vehicleType),
                            style: const TextStyle(fontSize: 12.5),
                            overflow: TextOverflow.ellipsis,
                          ),
                        ),
                        Text(
                          '${kmh(p.speed)} ${l.kmh} · ${fmtDist(l, p.distance)}',
                          style: const TextStyle(fontSize: 12.5),
                        ),
                      ],
                    ),
                  ),
              ],
            ),
          ),
        ],
      ),
    );
  }
}
