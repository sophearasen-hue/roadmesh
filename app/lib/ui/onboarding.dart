/// First run (SPEC §9 "Onboarding", AUDIT L-07): language first, then vehicle type, then
/// the safety notice and privacy summary, which the driver must accept once. Permissions
/// are asked when the first drive starts. Never shown while moving (it runs before any
/// drive starts).
library;

import 'package:flutter/material.dart';

import '../l10n/app_localizations.dart';
import 'controller.dart';
import 'style.dart';
import 'widgets.dart';

/// Vehicle types offered (codes as in the safety message).
const vehicleChoices = [1, 0, 2, 3];

/// The onboarding flow.
class OnboardingScreen extends StatefulWidget {
  /// State.
  const OnboardingScreen({super.key, required this.c});

  /// State.
  final DriveController c;

  @override
  State<OnboardingScreen> createState() => _OnboardingScreenState();
}

class _OnboardingScreenState extends State<OnboardingScreen> {
  int _page = 0;

  @override
  Widget build(BuildContext context) {
    final l = AppLocalizations.of(context);
    final c = widget.c;
    final lang =
        c.locale?.languageCode ?? Localizations.localeOf(context).languageCode;
    final pages = [
      _Page(
        title: l.onboardLanguage,
        body: Chips(
          const ['English', 'ខ្មែរ'],
          selected: lang == 'km' ? 1 : 0,
          onTap: (i) => c.setLanguage(i == 1 ? 'km' : 'en'),
          keys: const [Key('ob-en'), Key('ob-km')],
        ),
      ),
      _Page(
        title: l.onboardVehicle,
        body: Chips(
          [for (final v in vehicleChoices) typeName(l, v)],
          selected: vehicleChoices.indexOf(c.vehicleType),
          onTap: (i) => c.setVehicleType(vehicleChoices[i]),
          keys: [for (final v in vehicleChoices) Key('ob-vehicle-$v')],
        ),
      ),
      _Page(
        title: l.safetyTitle,
        body: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Text(
              l.safetyText,
              key: const Key('safetyText'),
              style: const TextStyle(fontSize: 14, height: 1.6),
            ),
            const SizedBox(height: 16),
            Text(
              l.privacy,
              style: const TextStyle(fontSize: 13, fontWeight: FontWeight.w600),
            ),
            const SizedBox(height: 6),
            Text(
              l.privacyTxt,
              style: const TextStyle(
                fontSize: 12.5,
                color: C.muted,
                height: 1.6,
              ),
            ),
          ],
        ),
      ),
    ];
    final last = _page == pages.length - 1;
    return Scaffold(
      backgroundColor: C.panel,
      body: SafeArea(
        child: Padding(
          padding: const EdgeInsets.fromLTRB(20, 24, 20, 20),
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              Text(
                l.onboardWelcome,
                style: const TextStyle(
                  fontSize: 22,
                  fontWeight: FontWeight.w700,
                ),
              ),
              const SizedBox(height: 4),
              Text(
                l.tagline,
                style: const TextStyle(fontSize: 13, color: C.muted),
              ),
              const SizedBox(height: 28),
              Expanded(child: SingleChildScrollView(child: pages[_page])),
              Row(
                children: [
                  for (var i = 0; i < pages.length; i++)
                    Container(
                      width: 8,
                      height: 8,
                      margin: const EdgeInsets.only(right: 6),
                      decoration: BoxDecoration(
                        shape: BoxShape.circle,
                        color: i == _page ? C.blue : C.line,
                      ),
                    ),
                  const Spacer(),
                  FilledButton(
                    key: Key(last ? 'ob-accept' : 'ob-next'),
                    onPressed: () =>
                        last ? c.finishOnboarding() : setState(() => _page++),
                    child: Text(last ? l.onboardAccept : l.onboardNext),
                  ),
                ],
              ),
            ],
          ),
        ),
      ),
    );
  }
}

class _Page extends StatelessWidget {
  const _Page({required this.title, required this.body});
  final String title;
  final Widget body;

  @override
  Widget build(BuildContext context) => Column(
    crossAxisAlignment: CrossAxisAlignment.start,
    children: [
      Text(
        title,
        style: const TextStyle(fontSize: 18, fontWeight: FontWeight.w600),
      ),
      const SizedBox(height: 16),
      body,
    ],
  );
}
