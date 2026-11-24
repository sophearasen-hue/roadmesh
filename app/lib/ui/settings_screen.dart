/// Settings tab (AUDIT P-23 to P-28).
library;

import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';

import '../l10n/app_localizations.dart';
import '../radio_test.dart';
import 'controller.dart';
import 'style.dart';
import 'widgets.dart';

/// The tab.
class SettingsScreen extends StatelessWidget {
  /// State.
  const SettingsScreen({super.key, required this.c});

  /// State.
  final DriveController c;

  @override
  Widget build(BuildContext context) {
    final l = AppLocalizations.of(context);
    final lang =
        c.locale?.languageCode ?? Localizations.localeOf(context).languageCode;
    Widget section(String title, List<Widget> children) => Container(
      margin: const EdgeInsets.only(bottom: 10),
      padding: const EdgeInsets.all(12),
      decoration: BoxDecoration(
        color: C.panel2,
        borderRadius: BorderRadius.circular(14),
      ),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Text(
            title,
            style: const TextStyle(fontSize: 13, fontWeight: FontWeight.w600),
          ),
          const SizedBox(height: 8),
          ...children,
        ],
      ),
    );
    Widget radio(String name, String desc, {required bool on}) => Container(
      margin: const EdgeInsets.only(bottom: 6),
      padding: const EdgeInsets.symmetric(horizontal: 10, vertical: 8),
      decoration: BoxDecoration(
        color: on ? const Color(0xFF13243A) : C.panel,
        borderRadius: BorderRadius.circular(10),
        border: Border.all(color: on ? C.blue : C.line),
      ),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Text(
            name,
            style: TextStyle(fontSize: 12.5, color: on ? C.text : C.muted),
          ),
          Text(desc, style: const TextStyle(fontSize: 11, color: C.muted)),
        ],
      ),
    );
    return ListView(
      padding: const EdgeInsets.fromLTRB(14, 14, 14, 20),
      children: [
        Text(
          l.tabSettings,
          style: const TextStyle(fontSize: 16, fontWeight: FontWeight.w600),
        ),
        const SizedBox(height: 10),
        section(l.language, [
          Chips(
            const ['English', 'ខ្មែរ'],
            selected: lang == 'km' ? 1 : 0,
            onTap: (i) => c.setLanguage(i == 1 ? 'km' : 'en'),
            keys: const [Key('lang-en'), Key('lang-km')],
          ),
        ]),
        section(l.radioMode, [
          radio(l.radioBle, l.radioBleD, on: true),
          radio(l.radioWifi, l.radioWifiD, on: false),
          radio(l.radioLora, l.radioLoraD, on: false),
          Text(
            l.radioSoon,
            style: const TextStyle(
              fontSize: 11.5,
              color: C.muted,
              height: 1.55,
            ),
          ),
        ]),
        // P-25: a debug setting only; release builds use the SPEC §5 TTLs
        if (kDebugMode)
          section(l.maxHops(c.debugMaxHops), [
            Slider(
              key: const Key('hops'),
              min: 1,
              max: 6,
              divisions: 5,
              value: c.debugMaxHops.toDouble(),
              onChanged: (v) => c.setDebugMaxHops(v.round()),
            ),
            Text(
              l.maxHopsD,
              style: const TextStyle(
                fontSize: 11.5,
                color: C.muted,
                height: 1.55,
              ),
            ),
          ]),
        section(l.alertSound, [
          Row(
            children: [
              Expanded(
                child: Text(
                  l.alertSoundD,
                  style: const TextStyle(fontSize: 12.5),
                ),
              ),
              Switch(
                key: const Key('sound'),
                value: !c.muted,
                onChanged: (on) => c.setMuted(!on),
              ),
            ],
          ),
        ]),
        section(l.motoMode, [
          Row(
            children: [
              Expanded(
                child: Text(
                  l.motoModeD,
                  style: const TextStyle(fontSize: 12.5),
                ),
              ),
              Switch(
                key: const Key('moto'),
                value: c.motorbikeMode,
                onChanged: c.setMotorbikeMode,
              ),
            ],
          ),
        ]),
        section(l.privacy, [
          Text(
            l.privacyTxt,
            key: const Key('privacyTxt'),
            style: const TextStyle(fontSize: 12, color: C.muted, height: 1.6),
          ),
        ]),
        section(l.deleteData, [
          Text(
            l.deleteDataD,
            style: const TextStyle(fontSize: 12, color: C.muted, height: 1.6),
          ),
          const SizedBox(height: 8),
          Align(
            alignment: Alignment.centerLeft,
            child: OutlinedButton(
              key: const Key('deleteData'),
              onPressed: () => _confirmDelete(context),
              style: OutlinedButton.styleFrom(
                foregroundColor: const Color(0xFFFCA5A5),
                side: const BorderSide(color: Color(0xFF7F1D1D)),
              ),
              child: Text(l.deleteData),
            ),
          ),
        ]),
        if (kDebugMode)
          section(l.debugTools, [
            Align(
              alignment: Alignment.centerLeft,
              child: OutlinedButton(
                onPressed: () => Navigator.of(context).push(
                  MaterialPageRoute<void>(
                    builder: (_) => const RadioTestScreen(),
                  ),
                ),
                child: Text(l.radioTest),
              ),
            ),
          ]),
      ],
    );
  }

  Future<void> _confirmDelete(BuildContext context) async {
    final l = AppLocalizations.of(context);
    final ok = await showDialog<bool>(
      context: context,
      builder: (ctx) => AlertDialog(
        content: Text(l.deleteConfirm),
        actions: [
          TextButton(
            onPressed: () => Navigator.pop(ctx, false),
            child: Text(l.cancel),
          ),
          TextButton(
            key: const Key('confirmDelete'),
            onPressed: () => Navigator.pop(ctx, true),
            child: Text(l.delete),
          ),
        ],
      ),
    );
    if (ok != true || !context.mounted) return;
    await c.deleteMyData();
    if (context.mounted) {
      ScaffoldMessenger.of(context)
          .showSnackBar(SnackBar(content: Text(l.deleteDone)));
    }
  }
}
