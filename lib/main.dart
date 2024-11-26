import 'package:flutter/material.dart';
import 'package:flutter_localizations/flutter_localizations.dart';
import 'package:lanmsg/gen/strings.g.dart';
import 'package:lanmsg/utils/platform.dart';

import 'screens/home_screen.dart';

void main() {
  WidgetsFlutterBinding.ensureInitialized();
  LocaleSettings.useDeviceLocale();
  runApp(TranslationProvider(child: const MyApp()));
}

class MyApp extends StatelessWidget {
  const MyApp({super.key});

  @override
  Widget build(BuildContext context) {
    final String? fontFamily;
    if (checkPlatform([TargetPlatform.windows])) {
      final locale = LocaleSettings.currentLocale;
      fontFamily = switch (locale) {
        AppLocale.ja => 'Yu Gothic UI',
        AppLocale.ko => 'Malgun Gothic',
        AppLocale.zhCn => 'Microsoft YaHei UI',
        AppLocale.zhHk || AppLocale.zhTw => 'Microsoft JhengHei UI',
        _ => 'Segoe UI Variable Display',
      };
    } else {
      fontFamily = null;
    }
    return MaterialApp(
      debugShowCheckedModeBanner: false,
      title: t.app_name,
      locale: TranslationProvider.of(context).flutterLocale,
      supportedLocales: AppLocaleUtils.supportedLocales,
      localizationsDelegates: GlobalMaterialLocalizations.delegates,
      theme: ThemeData(
        colorScheme: ColorScheme.fromSeed(
          seedColor: const Color(0xFF07C160),
        ),
        useMaterial3: true,
        textTheme: const TextTheme(
          bodyLarge: TextStyle(fontSize: 14),
          bodyMedium: TextStyle(fontSize: 13),
          bodySmall: TextStyle(fontSize: 12),
          labelLarge: TextStyle(fontSize: 14),
          labelMedium: TextStyle(fontSize: 13),
          labelSmall: TextStyle(fontSize: 12),
        ),
        fontFamily: fontFamily,
      ),
      home: const HomeScreen(),
    );
  }
}
