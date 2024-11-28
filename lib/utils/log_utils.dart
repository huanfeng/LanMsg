import 'dart:developer' as developer;

import 'package:logging/logging.dart';

class LogUtils {
  static void initLogger() {
    Logger.root.level = Level.ALL;
    Logger.root.onRecord.listen((record) {
      developer.log(
          '${record.time} [${record.level.name}] ${record.loggerName}: ${record.message}');
    });
  }
}
