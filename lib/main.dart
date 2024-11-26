import 'package:flutter/material.dart';
import 'package:flutter_localizations/flutter_localizations.dart';
import 'package:lanmsg/config/theme.dart';
import 'package:lanmsg/gen/strings.g.dart';
import 'package:lanmsg/services/message_service.dart';
import 'package:lanmsg/utils/platform_utils.dart';
import 'package:provider/provider.dart';
import 'package:window_manager/window_manager.dart';

import 'config/settings.dart';
import 'screens/home_screen.dart';

void main() async {
  WidgetsFlutterBinding.ensureInitialized();

  final messageService = MessageService();
  await messageService.init();

  // 生成一些模拟数据
  await messageService.generateSimulatedMessages('chat1', count: 20);

  if (PlatformUtils.isDesktop()) {
    await windowManager.ensureInitialized();

    WindowOptions windowOptions = WindowOptions(
      // size: Size(1024, 768),
      // minimumSize: const Size(800, 600),
      backgroundColor: Colors.transparent,
      skipTaskbar: false,
      center: true,
      title: t.app_name,
      titleBarStyle: TitleBarStyle.normal,
    );

    await windowManager.waitUntilReadyToShow(windowOptions);
    await SettingsService.init();

    // 恢复上次保存的窗口大小
    final savedSize = SettingsService.getWindowSize();
    if (savedSize != null) {
      await windowManager.setSize(savedSize);
    }

    await windowManager.show();
    await windowManager.focus();
  }
  LocaleSettings.useDeviceLocale();
  runApp(
    Provider<MessageService>.value(
      value: messageService,
      child: TranslationProvider(child: const MyApp()),
    ),
  );
}

class MyApp extends StatelessWidget {
  const MyApp({super.key});

  @override
  Widget build(BuildContext context) {
    return MaterialApp(
      debugShowCheckedModeBanner: false,
      title: t.app_name,
      locale: TranslationProvider.of(context).flutterLocale,
      supportedLocales: AppLocaleUtils.supportedLocales,
      localizationsDelegates: GlobalMaterialLocalizations.delegates,
      theme: getTheme(Brightness.light),
      home: const HomeScreen(),
    );
  }
}
