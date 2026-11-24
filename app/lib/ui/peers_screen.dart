/// Peers tab (AUDIT P-12 to P-15): the vehicles in my mesh and one vehicle's detail.
library;

import 'package:flutter/material.dart';

import '../l10n/app_localizations.dart';
import '../radio.dart';
import 'controller.dart';
import 'drive_screen.dart' show emptyUpdate;
import 'style.dart';
import 'widgets.dart';

/// List or detail.
class PeersScreen extends StatelessWidget {
  /// State.
  const PeersScreen({super.key, required this.c});

  /// State.
  final DriveController c;

  @override
  Widget build(BuildContext context) {
    final l = AppLocalizations.of(context);
    final u = c.last ?? emptyUpdate;
    final sel = c.selected;
    if (sel != null) {
      for (final p in u.peers) {
        if (p.id == sel) return PeerDetail(c: c, peer: p);
      }
    }
    final list = [...u.peers]..sort((a, b) => a.distance.compareTo(b.distance));
    return ListView(
      padding: const EdgeInsets.fromLTRB(14, 14, 14, 20),
      children: [
        Text(
          l.peersTitle(list.length),
          key: const Key('peersTitle'),
          style: const TextStyle(fontSize: 16, fontWeight: FontWeight.w600),
        ),
        const SizedBox(height: 6),
        Text(
          '${l.peersSub} · ${l.msgRate}',
          style: const TextStyle(fontSize: 12, color: C.muted, height: 1.6),
        ),
        const SizedBox(height: 10),
        if (list.isEmpty) EmptyText(l.peersEmpty),
        for (final p in list) ...[
          PeerCard(peer: p, onTap: () => c.openPeer(p.id)),
          const SizedBox(height: 8),
        ],
      ],
    );
  }
}

/// One row of the list (P-13).
class PeerCard extends StatelessWidget {
  /// Peer and tap handler.
  const PeerCard({super.key, required this.peer, required this.onTap});

  /// Peer.
  final Peer peer;

  /// Opens the detail.
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    final l = AppLocalizations.of(context);
    final b = band(peer.score);
    return InkWell(
      key: Key('peer-${peer.id}'),
      onTap: onTap,
      borderRadius: BorderRadius.circular(14),
      child: Container(
        padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 10),
        decoration: BoxDecoration(
          color: C.panel2,
          borderRadius: BorderRadius.circular(14),
          border: Border.all(color: C.line),
        ),
        child: Row(
          children: [
            TypeTile(peer.vehicleType),
            const SizedBox(width: 10),
            Expanded(
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  Text.rich(
                    TextSpan(
                      text: '${typeName(l, peer.vehicleType)} ',
                      style: const TextStyle(
                        fontSize: 13.5,
                        fontWeight: FontWeight.w600,
                      ),
                      children: [
                        TextSpan(
                          text: pseudonymLabel(peer.id),
                          style: const TextStyle(
                            fontSize: 11,
                            fontWeight: FontWeight.w400,
                            color: C.muted,
                          ),
                        ),
                      ],
                    ),
                  ),
                  const SizedBox(height: 2),
                  Text(
                    '${fmtDist(l, peer.distance)} · ${linkText(l, peer.hops)} · ${l.unknownRoad}',
                    style: const TextStyle(fontSize: 11.5, color: C.muted),
                  ),
                  RiskBadge('${bandName(l, b)} · ${peer.score}', b),
                ],
              ),
            ),
            Column(
              crossAxisAlignment: CrossAxisAlignment.end,
              children: [
                Text(
                  '${kmh(peer.speed)}',
                  style: const TextStyle(
                    fontSize: 16,
                    fontWeight: FontWeight.w700,
                  ),
                ),
                Text(
                  l.kmh,
                  style: const TextStyle(fontSize: 11.5, color: C.muted),
                ),
              ],
            ),
          ],
        ),
      ),
    );
  }
}

/// The on-screen form of a pseudonym (prototype "RM-1A2B"): its first 2 bytes. It changes
/// with the pseudonym; it is never logged or stored.
String pseudonymLabel(String hexId) =>
    'RM-${(hexId.length >= 4 ? hexId.substring(0, 4) : hexId).toUpperCase()}';

/// One vehicle (P-15).
class PeerDetail extends StatelessWidget {
  /// State and the peer.
  const PeerDetail({super.key, required this.c, required this.peer});

  /// State.
  final DriveController c;

  /// Peer.
  final Peer peer;

  @override
  Widget build(BuildContext context) {
    final l = AppLocalizations.of(context);
    final u = c.last ?? emptyUpdate;
    final b = band(peer.score);
    final events = u.events.where((e) => !e.me && e.id == peer.id).toList();
    final h = c.peerHistory[peer.id] ?? const <int>[];
    return ListView(
      key: const Key('peerDetail'),
      padding: const EdgeInsets.fromLTRB(14, 14, 14, 20),
      children: [
        Align(
          alignment: Alignment.centerLeft,
          child: TextButton(
            key: const Key('back'),
            onPressed: c.closePeer,
            style: TextButton.styleFrom(
              padding: EdgeInsets.zero,
              foregroundColor: C.tabOn,
            ),
            child: Text(l.back, style: const TextStyle(fontSize: 13)),
          ),
        ),
        Row(
          children: [
            TypeTile(peer.vehicleType, size: 44),
            const SizedBox(width: 10),
            Expanded(
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  Text(
                    '${typeName(l, peer.vehicleType)} · ${pseudonymLabel(peer.id)}',
                    style: const TextStyle(
                      fontSize: 16,
                      fontWeight: FontWeight.w600,
                    ),
                  ),
                  RiskBadge('${bandName(l, b)} · ${l.score} ${peer.score}', b),
                ],
              ),
            ),
          ],
        ),
        const SizedBox(height: 12),
        KvGrid([
          (l.speed, '${kmh(peer.speed)} ${l.kmh}'),
          (l.accel, '${peer.accel.toStringAsFixed(1)} m/s²'),
          (l.dist, fmtDist(l, peer.distance)),
          (l.link, linkText(l, peer.hops)),
          (l.road, l.unknownRoad),
          (l.latency, '${peer.ageMs} ms'),
        ]),
        SectionTitle(l.speedHistory),
        SpeedGraph(h, limitKmh: u.own.limit == null ? null : kmh(u.own.limit!)),
        SectionTitle(l.events),
        Chips([
          for (final k in eventKeys)
            '${eventName(l, k)} · ${peer.counts.of(k)}',
        ]),
        const SizedBox(height: 6),
        if (events.isEmpty) EmptyText(l.noEvents),
        for (final e in events)
          EventRow(
            ago: l.ago(e.age.round()),
            kind: e.kind,
            who:
                '${typeName(l, peer.vehicleType)} ${pseudonymLabel(e.id)} · ${e.kmh.round()} ${l.kmh}',
          ),
      ],
    );
  }
}
