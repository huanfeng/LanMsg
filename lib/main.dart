import 'package:flutter/material.dart';
import 'package:flutter_localizations/flutter_localizations.dart';
import 'package:lanmsg/config/theme.dart';
import 'package:lanmsg/gen/strings.g.dart';
import 'package:lanmsg/services/message_service.dart';
import 'package:lanmsg/utils/log_utils.dart';
import 'package:lanmsg/utils/platform_utils.dart';
import 'package:provider/provider.dart';
import 'package:window_manager/window_manager.dart';

import 'config/settings.dart';
import 'screens/home_screen.dart';

void main() async {
  WidgetsFlutterBinding.ensureInitialized();

  LogUtils.initLogger();

  final messageService = MessageService();
  await messageService.init();
  // await messageService.clearMessages(); // 清理旧数据

  // 生成一些模拟数据
  // await Future.wait([
  //   messageService.generateSimulatedMessages(
  //     'chat1',
  //     count: 20,
  //     name: '张三',
  //     avatar: 'https://via.placeholder.com/50?text=张三',
  //   ),
  //   messageService.generateSimulatedMessages(
  //     'chat2',
  //     count: 15,
  //     name: '李四',
  //     avatar: 'https://via.placeholder.com/50?text=李四',
  //   ),
  //   messageService.generateSimulatedMessages(
  //     'chat3',
  //     count: 10,
  //     name: '王五',
  //     avatar: 'https://via.placeholder.com/50?text=王五',
  //   ),
  // ]);

  await SettingsService.init();

  if (PlatformUtils.isDesktop()) {
    await windowManager.ensureInitialized();

    final windowSize = SettingsService.getWindowSize();

    WindowOptions windowOptions = WindowOptions(
      size: windowSize,
      // minimumSize: const Size(800, 600),
      backgroundColor: Colors.transparent,
      skipTaskbar: false,
      // center: true,
      title: t.app_name,
      titleBarStyle: TitleBarStyle.normal,
    );

    windowManager.waitUntilReadyToShow(windowOptions);
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
