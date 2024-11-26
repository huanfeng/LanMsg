import 'dart:io';
import 'dart:math';

import 'package:lanmsg/config/config.dart';
import 'package:lanmsg/models/message.dart';
import 'package:lanmsg/objectbox.g.dart';
import 'package:path/path.dart' as path;

class MessageService {
  late final Store _store;
  late final Box<Message> _messageBox;

  Future<void> init() async {
    final docsDir = await Config.getStorageLocation();
    final dir = Directory(docsDir);
    if (!dir.existsSync()) {
      dir.createSync(recursive: true);
    }
    _store = await openStore(directory: path.join(docsDir, 'db'));
    _messageBox = _store.box<Message>();
  }

  Future<void> saveMessage(Message message) async {
    _messageBox.put(message);
  }

  List<Message> getMessagesForChat(String chatId) {
    return _messageBox.query(Message_.chatId.equals(chatId)).build().find();
  }

  Future<void> markAsRead(String messageId) async {
    final message = _messageBox
        .query(Message_.messageId.equals(messageId))
        .build()
        .findFirst();
    if (message != null) {
      final updatedMessage = message.copyWith(isRead: true);
      _messageBox.put(updatedMessage);
    }
  }

  // 模拟生成消息数据
  Future<void> generateSimulatedMessages(String chatId,
      {int count = 10}) async {
    final random = Random();
    final senders = ['user1', 'user2']; // 模拟两个用户
    final messages = [
      '你好！',
      '最近怎么样？',
      '在忙什么呢？',
      '今天天气不错',
      '要一起出去玩吗？',
      '好的，没问题',
      '改天再聊',
    ];

    for (var i = 0; i < count; i++) {
      final message = Message(
        messageId:
            DateTime.now().millisecondsSinceEpoch.toString() + i.toString(),
        chatId: chatId,
        senderId: senders[random.nextInt(senders.length)],
        content: messages[random.nextInt(messages.length)],
        timestamp:
            DateTime.now().subtract(Duration(minutes: random.nextInt(60))),
      );
      await saveMessage(message);
    }
  }

  void dispose() {
    _store.close();
  }
}
