import 'package:flutter/material.dart';

import 'l10n/app_localizations.dart';

void main() {
  runApp(const RoadMeshApp());
}

/// Root widget. Placeholder screen until BUILD_PLAN Step 8.
///
/// All user-facing text comes from `lib/l10n/app_*.arb` (CLAUDE.md "Localisation").
class RoadMeshApp extends StatelessWidget {
  const RoadMeshApp({super.key, this.locale});

  /// Forced locale for tests and for the in-app language switch; null = device locale.
  final Locale? locale;

  @override
  Widget build(BuildContext context) {
    return MaterialApp(
      onGenerateTitle: (context) => AppLocalizations.of(context).appTitle,
      locale: locale,
      localizationsDelegates: AppLocalizations.localizationsDelegates,
      supportedLocales: AppLocalizations.supportedLocales,
      home: const _PlaceholderHome(),
    );
  }
}

class _PlaceholderHome extends StatelessWidget {
  const _PlaceholderHome();

  @override
  Widget build(BuildContext context) {
    final l10n = AppLocalizations.of(context);
    return Scaffold(
      body: Center(
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            Text(l10n.appTitle, style: Theme.of(context).textTheme.headlineMedium),
            const SizedBox(height: 8),
            Text(l10n.tagline, textAlign: TextAlign.center),
          ],
        ),
      ),
    );
  }
}
