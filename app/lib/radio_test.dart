import 'dart:async';

import 'package:flutter/material.dart';

import 'radio.dart';

import 'l10n/app_localizations.dart';

/// Debug-only field-test screen (BUILD_PLAN Step 5): start a drive, enrol with the
/// developer CA, and see every phone heard with its speed and distance.
class RadioTestScreen extends StatefulWidget {
  const RadioTestScreen({super.key});

  @override
  State<RadioTestScreen> createState() => _RadioTestScreenState();
}

class _RadioTestScreenState extends State<RadioTestScreen> {
  StreamSubscription<RadioUpdate>? _sub;
  RadioUpdate? _last;
  Map<Object?, Object?> _status = const {};
  String? _message;

  @override
  void dispose() {
    _sub?.cancel();
    super.dispose();
  }

  Future<void> _refresh() async {
    final s = await RoadMeshRadio.status();
    if (mounted) setState(() => _status = s);
  }

  Future<void> _start(AppLocalizations l10n) async {
    if (!await RoadMeshRadio.requestPermissions()) {
      setState(() => _message = l10n.radioPermissionDenied);
      return;
    }
    await RoadMeshRadio.start(
      notificationTitle: l10n.notifTitle,
      notificationText: l10n.notifText,
    );
    _sub ??= RoadMeshRadio.updates.listen((u) => setState(() => _last = u));
    await Future<void>.delayed(const Duration(seconds: 2));
    await _refresh();
  }

  Future<void> _stop() async {
    await _sub?.cancel();
    _sub = null;
    await RoadMeshRadio.stop();
    await _refresh();
  }

  Future<void> _saved(AppLocalizations l10n, Future<String?> f) async {
    final path = await f;
    if (path != null && mounted) {
      setState(() => _message = l10n.radioSavedTo(path));
    }
  }

  String _type(AppLocalizations l10n, int code) => switch (code) {
    0 => l10n.typeCar,
    1 => l10n.typeMoto,
    2 => l10n.typeTuk,
    3 => l10n.typeTruck,
    _ => l10n.typeOther,
  };

  @override
  Widget build(BuildContext context) {
    final l10n = AppLocalizations.of(context);
    final peers = _last?.peers ?? const <Peer>[];
    final canSend = _status['canSend'] == true;
    return Scaffold(
      appBar: AppBar(title: Text(l10n.radioTest)),
      body: ListView(
        padding: const EdgeInsets.all(16),
        children: [
          Wrap(
            spacing: 8,
            runSpacing: 8,
            children: [
              FilledButton(
                onPressed: () => _start(l10n),
                child: Text(l10n.radioStart),
              ),
              OutlinedButton(onPressed: _stop, child: Text(l10n.radioStop)),
              OutlinedButton(
                onPressed: () =>
                    _saved(l10n, RoadMeshRadio.createCertificateRequest()),
                child: Text(l10n.radioRequest),
              ),
              OutlinedButton(
                onPressed: () async {
                  await RoadMeshRadio.importCertificates();
                  await _refresh();
                },
                child: Text(l10n.radioImport),
              ),
              OutlinedButton(
                onPressed: () => _saved(l10n, RoadMeshRadio.exportFieldLog()),
                child: Text(l10n.radioExportLog),
              ),
            ],
          ),
          const SizedBox(height: 12),
          Text(l10n.radioCertificates((_status['certificates'] as int?) ?? 0)),
          Text(
            l10n.radioCodedPhy(
              _status['codedPhy'] == true ? l10n.yes : l10n.no,
            ),
          ),
          if (_status['running'] == true && !canSend)
            Text(l10n.radioCannotSend),
          if (_message != null) Text(_message!),
          const Divider(),
          if (peers.isEmpty) Text(l10n.radioNoPeers),
          for (final p in peers)
            ListTile(
              dense: true,
              title: Text(
                l10n.radioPeer(
                  _type(l10n, p.vehicleType),
                  p.distance.round(),
                  (p.speed * 3.6).round(),
                ),
              ),
            ),
        ],
      ),
    );
  }
}
