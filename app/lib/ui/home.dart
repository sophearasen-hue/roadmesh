/// App shell: header, the four tabs (Drive, Peers, Behaviour, Settings) and, on a wide
/// landscape screen, the CarPlay-style layout (map + side panel, no tabs).
library;

import 'package:flutter/material.dart';

import '../l10n/app_localizations.dart';
import 'behaviour_screen.dart';
import 'controller.dart';
import 'drive_screen.dart';
import 'peers_screen.dart';
import 'settings_screen.dart';
import 'style.dart';

/// Width from which the CarPlay layout is used (prototype: 960 × 480).
const carPlayMinWidth = 800.0;

/// The shell.
class HomeScreen extends StatelessWidget {
  /// State; `onDrive` starts or stops the drive service (null hides the button).
  const HomeScreen({super.key, required this.c, this.driving, this.onDrive});

  /// State.
  final DriveController c;

  /// Whether a drive is running (null = unknown/demo).
  final bool? driving;

  /// Start/stop.
  final VoidCallback? onDrive;

  @override
  Widget build(BuildContext context) {
    return LayoutBuilder(
      builder: (context, box) {
        final carplay =
            box.maxWidth >= carPlayMinWidth && box.maxWidth > box.maxHeight;
        final body = carplay
            ? DriveScreen(c: c, carplay: true)
            : switch (c.tab) {
                AppTab.drive => DriveScreen(c: c),
                AppTab.peers => PeersScreen(c: c),
                AppTab.behaviour => BehaviourScreen(c: c),
                AppTab.settings => SettingsScreen(c: c),
              };
        return Scaffold(
          backgroundColor: C.panel,
          body: SafeArea(
            child: Column(
              children: [
                _Header(c: c, driving: driving, onDrive: onDrive),
                Expanded(child: body),
                if (!carplay) _Tabs(c: c),
              ],
            ),
          ),
        );
      },
    );
  }
}

class _Header extends StatelessWidget {
  const _Header({required this.c, this.driving, this.onDrive});
  final DriveController c;
  final bool? driving;
  final VoidCallback? onDrive;

  @override
  Widget build(BuildContext context) {
    final l = AppLocalizations.of(context);
    final km =
        (c.locale?.languageCode ??
            Localizations.localeOf(context).languageCode) ==
        'km';
    Widget pill(
      String text,
      VoidCallback? onTap, {
      Key? key,
      bool on = false,
    }) => InkWell(
      key: key,
      onTap: onTap,
      borderRadius: BorderRadius.circular(999),
      child: Container(
        padding: const EdgeInsets.symmetric(horizontal: 11, vertical: 5),
        decoration: BoxDecoration(
          color: C.panel2,
          borderRadius: BorderRadius.circular(999),
          border: Border.all(color: on ? C.blue : C.line),
        ),
        child: Text(
          text,
          style: TextStyle(
            fontSize: 12,
            color: on ? const Color(0xFFBCD5FF) : C.text,
          ),
        ),
      ),
    );
    return Container(
      padding: const EdgeInsets.fromLTRB(16, 12, 16, 10),
      decoration: const BoxDecoration(
        border: Border(bottom: BorderSide(color: C.line)),
      ),
      child: Row(
        children: [
          Container(
            width: 26,
            height: 26,
            decoration: BoxDecoration(
              borderRadius: BorderRadius.circular(8),
              gradient: const LinearGradient(colors: [C.blue, C.safe]),
            ),
            padding: const EdgeInsets.all(8),
            child: Container(
              decoration: BoxDecoration(
                shape: BoxShape.circle,
                border: Border.all(color: Colors.white, width: 2),
              ),
            ),
          ),
          const SizedBox(width: 8),
          Expanded(
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Text(
                  l.appTitle,
                  style: const TextStyle(fontWeight: FontWeight.w700),
                ),
                Text(
                  l.tagline,
                  style: const TextStyle(fontSize: 10.5, color: C.muted),
                  overflow: TextOverflow.ellipsis,
                ),
              ],
            ),
          ),
          pill(
            km ? 'EN' : 'ខ្មែរ',
            () => c.setLanguage(km ? 'en' : 'km'),
            key: const Key('langPill'),
          ),
          if (onDrive != null) ...[
            const SizedBox(width: 6),
            pill(
              driving == true ? l.driveStop : l.driveStart,
              onDrive,
              key: const Key('drivePill'),
              on: driving == true,
            ),
          ],
        ],
      ),
    );
  }
}

class _Tabs extends StatelessWidget {
  const _Tabs({required this.c});
  final DriveController c;

  @override
  Widget build(BuildContext context) {
    final l = AppLocalizations.of(context);
    final items = [
      (AppTab.drive, Icons.navigation_outlined, l.tabDrive),
      (AppTab.peers, Icons.wifi_tethering, l.tabPeers),
      (AppTab.behaviour, Icons.show_chart, l.tabBehavior),
      (AppTab.settings, Icons.settings_outlined, l.tabSettings),
    ];
    return Container(
      decoration: const BoxDecoration(
        color: C.panel,
        border: Border(top: BorderSide(color: C.line)),
      ),
      child: Row(
        children: [
          for (final (tab, icon, label) in items)
            Expanded(
              child: InkWell(
                key: Key('tab-${tab.name}'),
                onTap: () {
                  if (!c.select(tab)) {
                    ScaffoldMessenger.of(context)
                      ..clearSnackBars()
                      ..showSnackBar(
                        SnackBar(content: Text(l.lockedWhileDriving)),
                      );
                  }
                },
                child: Padding(
                  padding: const EdgeInsets.fromLTRB(2, 9, 2, 12),
                  child: Column(
                    children: [
                      Icon(
                        icon,
                        size: 22,
                        color: c.tab == tab
                            ? C.tabOn
                            : (c.locked && tab != AppTab.drive
                                  ? C.line
                                  : C.muted),
                      ),
                      const SizedBox(height: 4),
                      Text(
                        label,
                        style: TextStyle(
                          fontSize: 11.5,
                          color: c.tab == tab ? C.tabOn : C.muted,
                        ),
                        maxLines: 1,
                        overflow: TextOverflow.ellipsis,
                      ),
                    ],
                  ),
                ),
              ),
            ),
        ],
      ),
    );
  }
}
