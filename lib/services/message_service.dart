import 'dart:convert';
import 'dart:io';
import 'dart:math';

import 'package:lanmsg/config/config.dart';
import 'package:lanmsg/models/message.dart';
import 'package:lanmsg/models/chat.dart';
import 'package:path/path.dart' as path;

class MessageService {
  late final String _dataDir;
  late final File _chatsFile;
  late final File _messagesFile;
  List<Chat> _chats = [];
  List<Message> _messages = [];

  Future<void> init() async {
    _dataDir = await Config.getStorageLocation();
    _chatsFile = File(path.join(_dataDir, 'chats.json'));
    _messagesFile = File(path.join(_dataDir, 'messages.json'));
    await _loadData();
  }

  Future<void> _loadData() async {
    try {
      if (await _chatsFile.exists()) {
        final chatsJson = await _chatsFile.readAsString();
        final chatsList = jsonDecode(chatsJson) as List;
        _chats = chatsList.map((e) => Chat.fromJson(e)).toList();
      }

      if (await _messagesFile.exists()) {
        final messagesJson = await _messagesFile.readAsString();
        final messagesList = jsonDecode(messagesJson) as List;
        _messages = messagesList.map((e) => Message.fromJson(e)).toList();
      }
    } catch (e) {
      print('Error loading data: $e');
      // 如果加载失败，使用空列表
      _chats = [];
      _messages = [];
    }
  }

  Future<void> _saveData() async {
    try {
      await _chatsFile.writeAsString(jsonEncode(_chats.map((e) => e.toJson()).toList()));
      await _messagesFile.writeAsString(jsonEncode(_messages.map((e) => e.toJson()).toList()));
    } catch (e) {
      print('Error saving data: $e');
    }
  }

  Future<void> clearAllData() async {
    _chats = [];
    _messages = [];
    await _saveData();
  }

  Future<void> saveMessage(Message message) async {
    _messages.add(message);

    // 更新或创建聊天
    final chatIndex = _chats.indexWhere((chat) => chat.chatId == message.chatId);
    if (chatIndex != -1) {
      final chat = _chats[chatIndex];
      final updatedChat = chat.copyWith(
        lastMessage: message.content,
        lastMessageTime: message.timestamp,
        unreadCount: message.senderId != 'user1'
            ? chat.unreadCount + 1
            : chat.unreadCount,
      );
      _chats[chatIndex] = updatedChat;
    }

    await _saveData();
  }

  List<Message> getMessagesForChat(String chatId) {
    return _messages
        .where((msg) => msg.chatId == chatId)
        .toList()
      ..sort((a, b) => a.timestamp.compareTo(b.timestamp));
  }

  List<Chat> getAllChats() {
    return _chats
      ..sort((a, b) => b.lastMessageTime.compareTo(a.lastMessageTime));
  }

  Future<void> markAsRead(String messageId) async {
    final messageIndex = _messages.indexWhere((msg) => msg.messageId == messageId);
    if (messageIndex != -1) {
      final message = _messages[messageIndex];
      final updatedMessage = message.copyWith(isRead: true);
      _messages[messageIndex] = updatedMessage;

      // 更新聊天的未读消息数
      final chatIndex = _chats.indexWhere((chat) => chat.chatId == message.chatId);
      if (chatIndex != -1) {
        final chat = _chats[chatIndex];
        if (chat.unreadCount > 0) {
          final updatedChat = chat.copyWith(unreadCount: chat.unreadCount - 1);
          _chats[chatIndex] = updatedChat;
        }
      }

      await _saveData();
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
    var chatIndex = _chats.indexWhere((chat) => chat.chatId == chatId);
    
    // 如果聊天不存在，创建新的聊天
    if (chatIndex == -1) {
      final chat = Chat(
        chatId: chatId,
        name: name ?? '联系人 $chatId',
        lastMessage: '',
        lastMessageTime: DateTime.now(),
        avatar: avatar ?? 'https://via.placeholder.com/50',
        unreadCount: 0,
      );
      _chats.add(chat);
      chatIndex = _chats.length - 1;
    }

    final baseTime = DateTime.now();
    for (var i = 0; i < count; i++) {
      final message = Message(
        messageId: '${chatId}_${DateTime.now().millisecondsSinceEpoch}_$i',
        chatId: chatId,
        senderId: senders[random.nextInt(senders.length)],
        content: messages[random.nextInt(messages.length)],
        timestamp: baseTime.subtract(Duration(minutes: i * 5 + random.nextInt(5))),
      );
      try {
        await saveMessage(message);
      } catch (e) {
        print('Error saving message: $e');
      }
    }
  }

  void dispose() {
    // No need to close the store as we are not using ObjectBox anymore
  }
}
