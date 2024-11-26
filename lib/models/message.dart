class Message {
  final String messageId;
  final String chatId;
  final String senderId;
  final String content;
  final DateTime timestamp;
  final bool isRead;

  Message({
    required this.messageId,
    required this.chatId,
    required this.senderId,
    required this.content,
    required this.timestamp,
    this.isRead = false,
  });

  Message copyWith({
    String? messageId,
    String? chatId,
    String? senderId,
    String? content,
    DateTime? timestamp,
    bool? isRead,
  }) {
    return Message(
      messageId: messageId ?? this.messageId,
      chatId: chatId ?? this.chatId,
      senderId: senderId ?? this.senderId,
      content: content ?? this.content,
      timestamp: timestamp ?? this.timestamp,
      isRead: isRead ?? this.isRead,
    );
  }

  Map<String, dynamic> toJson() => {
        'messageId': messageId,
        'chatId': chatId,
        'senderId': senderId,
        'content': content,
        'timestamp': timestamp.toIso8601String(),
        'isRead': isRead,
      };

  factory Message.fromJson(Map<String, dynamic> json) => Message(
        messageId: json['messageId'] as String,
        chatId: json['chatId'] as String,
        senderId: json['senderId'] as String,
        content: json['content'] as String,
        timestamp: DateTime.parse(json['timestamp'] as String),
        isRead: json['isRead'] as bool,
      );
}
