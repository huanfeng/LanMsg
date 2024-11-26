import 'dart:math';
import 'package:hive/hive.dart';
import 'package:lanmsg/models/message.dart';

class MessageService {
  static const String _boxName = 'messages';
  late Box<Message> _messageBox;
  
  Future<void> init() async {
    _messageBox = await Hive.openBox<Message>(_boxName);
  }

  Future<void> saveMessage(Message message) async {
    await _messageBox.put(message.id, message);
  }

  Future<List<Message>> getMessagesForChat(String chatId) async {
    return _messageBox.values
        .where((message) => message.chatId == chatId)
        .toList();
  }

  Future<void> markAsRead(String messageId) async {
    final message = _messageBox.get(messageId);
    if (message != null) {
      final updatedMessage = message.copyWith(isRead: true);
      await _messageBox.put(messageId, updatedMessage);
    }
  }

  // 模拟生成消息数据
  Future<void> generateSimulatedMessages(String chatId, {int count = 10}) async {
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
      '晚安',
      '早上好',
      '中午吃什么？',
    ];

    final now = DateTime.now();
    
    for (var i = 0; i < count; i++) {
      final message = Message(
        id: 'msg_${DateTime.now().millisecondsSinceEpoch}_$i',
        chatId: chatId,
        senderId: senders[random.nextInt(2)],
        content: messages[random.nextInt(messages.length)],
        timestamp: now.subtract(Duration(minutes: random.nextInt(60))),
        isRead: random.nextBool(),
      );
      
      await saveMessage(message);
    }
  }

  Future<void> clearMessages() async {
    await _messageBox.clear();
  }
}
