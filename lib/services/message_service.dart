import 'dart:io';
import 'dart:math';

import 'package:lanmsg/config/config.dart';
import 'package:lanmsg/models/message.dart';
import 'package:lanmsg/models/chat.dart';
import 'package:lanmsg/objectbox.g.dart';
import 'package:path/path.dart' as path;

class MessageService {
  late final Store _store;
  late final Box<Message> _messageBox;
  late final Box<Chat> _chatBox;

  Future<void> init() async {
    final docsDir = await Config.getStorageLocation();
    final dir = Directory(docsDir);
    if (!dir.existsSync()) {
      dir.createSync(recursive: true);
    }
    _store = await openStore(directory: path.join(docsDir, 'db'));
    _messageBox = _store.box<Message>();
    _chatBox = _store.box<Chat>();
    await clearAllData(); // 在初始化时调用清理数据的方法
  }

  // 清理所有数据
  Future<void> clearAllData() async {
    _messageBox.removeAll();
    _chatBox.removeAll();
  }

  Future<void> saveMessage(Message message) async {
    // 更新或创建聊天
    final chat =
        _chatBox.query(Chat_.chatId.equals(message.chatId)).build().findFirst();

    if (chat != null) {
      final updatedChat = chat.copyWith(
        lastMessage: message.content,
        lastMessageTime: message.timestamp,
        unreadCount: message.senderId != 'user1'
            ? chat.unreadCount + 1
            : chat.unreadCount,
      );
      _chatBox.put(updatedChat);
    }
  }

  List<Message> getMessagesForChat(String chatId) {
    return _messageBox
        .query(Message_.chatId.equals(chatId))
        .order(Message_.timestamp)
        .build()
        .find();
  }

  List<Chat> getAllChats() {
    return _chatBox
        .query()
        .order(Chat_.lastMessageTime, flags: Order.descending)
        .build()
        .find();
  }

  Future<void> markAsRead(String messageId) async {
    final message = _messageBox
        .query(Message_.messageId.equals(messageId))
        .build()
        .findFirst();
    if (message != null) {
      final updatedMessage = message.copyWith(isRead: true);
      _messageBox.put(updatedMessage);

      // 更新聊天的未读消息数
      final chat = _chatBox
          .query(Chat_.chatId.equals(message.chatId))
          .build()
          .findFirst();
      if (chat != null && chat.unreadCount > 0) {
        final updatedChat = chat.copyWith(unreadCount: chat.unreadCount - 1);
        _chatBox.put(updatedChat);
      }
    }
  }

  // 模拟生成消息数据
  Future<void> generateSimulatedMessages(
    String chatId, {
    int count = 10,
    String? name,
    String? avatar,
  }) async {
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

    // 检查聊天是否已存在
    var chat = _chatBox.query(Chat_.chatId.equals(chatId)).build().findFirst();

    // 如果聊天不存在，创建新的聊天
    if (chat == null) {
      chat = Chat(
        chatId: chatId,
        name: name ?? '联系人 $chatId',
        lastMessage: '',
        lastMessageTime: DateTime.now(),
        avatar: avatar ?? 'https://via.placeholder.com/50',
        unreadCount: 0,
      );
      _chatBox.put(chat);
    }

    final baseTime = DateTime.now();
    for (var i = 0; i < count; i++) {
      final message = Message(
        messageId: '${chatId}_${DateTime.now().millisecondsSinceEpoch}_$i',
        chatId: chatId,
        senderId: senders[random.nextInt(senders.length)],
        content: messages[random.nextInt(messages.length)],
        timestamp:
            baseTime.subtract(Duration(minutes: i * 5 + random.nextInt(5))),
      );
      try {
        await saveMessage(message);
      } catch (e) {
        print('Error saving message: $e');
      }
    }
  }

  void dispose() {
    _store.close();
  }
}
