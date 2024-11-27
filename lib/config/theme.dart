import 'package:flutter/material.dart';
import 'package:lanmsg/gen/strings.g.dart';
import 'package:lanmsg/utils/platform_utils.dart';

ThemeData getTheme(Brightness brightness) {
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

  return ThemeData(
    brightness: brightness,
    colorScheme: ColorScheme.fromSeed(
      seedColor: const Color(0xFF0084FF),
      brightness: brightness,
    ),
    scaffoldBackgroundColor: Colors.white,
    dividerColor: const Color(0xFFE6E6E6),
    textTheme: const TextTheme(
      bodyLarge: TextStyle(fontSize: 14),
      bodyMedium: TextStyle(fontSize: 13),
      bodySmall: TextStyle(fontSize: 12),
      labelLarge: TextStyle(fontSize: 14),
      labelMedium: TextStyle(fontSize: 13),
      labelSmall: TextStyle(fontSize: 12),
    ),
    useMaterial3: true,
    fontFamily: fontFamily,
  );
}
