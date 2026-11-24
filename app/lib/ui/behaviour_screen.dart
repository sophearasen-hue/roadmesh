/// Behaviour tab (AUDIT P-17 to P-22): my trip score, stats, events, speed graph, the
/// drivers around me and the event timeline.
library;

import 'package:flutter/material.dart';

import '../l10n/app_localizations.dart';
import 'controller.dart';
import 'drive_screen.dart' show emptyUpdate;
import 'peers_screen.dart' show pseudonymLabel;
import 'style.dart';
import 'widgets.dart';

/// The tab.
class BehaviourScreen extends StatelessWidget {
  /// State.
  const BehaviourScreen({super.key, required this.c});

  /// State.
  final DriveController c;

  @override
  Widget build(BuildContext context) {
    final l = AppLocalizations.of(context);
    final u = c.last ?? emptyUpdate;
    final t = u.trip;
    final b = band(t.score);
    var safe = 0, caution = 0, risky = 0;
    final types = <String, int>{};
    for (final p in u.peers) {
      types[p.id] = p.vehicleType;
      switch (band(p.score)) {
        case Band.safe:
          safe++;
        case Band.caution:
          caution++;
        case Band.risky:
          risky++;
      }
    }
    final ev = u.events
        .where((e) => c.filter == 'all' || (c.filter == 'me' ? e.me : !e.me))
        .take(40)
        .toList();
    return ListView(
      padding: const EdgeInsets.fromLTRB(14, 14, 14, 20),
      children: [
        Text(
          l.myTrip,
          style: const TextStyle(fontSize: 16, fontWeight: FontWeight.w600),
        ),
        const SizedBox(height: 10),
        Container(
          padding: const EdgeInsets.all(12),
          decoration: BoxDecoration(
            color: C.panel2,
            borderRadius: BorderRadius.circular(14),
          ),
          child: Row(
            children: [
              ScoreRing(t.score, key: const Key('scoreRing')),
              const SizedBox(width: 14),
              Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  Text(
                    l.drivingScore,
                    style: const TextStyle(fontSize: 13, color: C.muted),
                  ),
                  Text(
                    bandName(l, b),
                    key: const Key('bandName'),
                    style: const TextStyle(
                      fontSize: 22,
                      fontWeight: FontWeight.w700,
                    ),
                  ),
                ],
              ),
            ],
          ),
        ),
        const SizedBox(height: 8),
        KvGrid([
          (l.distance, fmtDist(l, t.distance)),
          (l.avgSpeed, '${kmh(t.avgSpeed)} ${l.kmh}'),
          (l.maxSpeed, '${kmh(t.maxSpeed)} ${l.kmh}'),
          (l.events, '${t.counts.total}'),
        ]),
        const SizedBox(height: 8),
        Chips([
          for (final k in eventKeys) '${eventName(l, k)} · ${t.counts.of(k)}',
        ]),
        SectionTitle(l.speedHistory),
        SpeedGraph(
          c.ownHistory,
          limitKmh: u.own.limit == null ? null : kmh(u.own.limit!),
        ),
        SectionTitle(l.networkSummary),
        Text(
          l.riskyPeers(risky, caution, safe),
          key: const Key('riskyPeers'),
          style: const TextStyle(fontSize: 12, color: C.muted),
        ),
        SectionTitle(l.timeline),
        Chips(
          [l.fAll, l.fMe, l.fPeers],
          selected: ['all', 'me', 'peers'].indexOf(c.filter),
          onTap: (i) => c.setFilter(['all', 'me', 'peers'][i]),
          keys: const [Key('f-all'), Key('f-me'), Key('f-peers')],
        ),
        const SizedBox(height: 6),
        if (ev.isEmpty) EmptyText(l.noEvents),
        for (final e in ev)
          EventRow(
            ago: l.ago(e.age.round()),
            kind: e.kind,
            who: e.me
                ? '${l.you} · ${e.kmh.round()} ${l.kmh}'
                : '${typeName(l, types[e.id] ?? 99)} ${pseudonymLabel(e.id)} · ${e.kmh.round()} ${l.kmh}',
          ),
      ],
    );
  }
}
