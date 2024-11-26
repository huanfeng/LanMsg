import 'package:flutter/material.dart';

class ChatDetail extends StatefulWidget {
  final String chatId;

  const ChatDetail({
    super.key,
    required this.chatId,
  });

  @override
  State<ChatDetail> createState() => _ChatDetailState();
}

class _ChatDetailState extends State<ChatDetail> {
  final TextEditingController _messageController = TextEditingController();
  final ScrollController _scrollController = ScrollController();

  final List<ChatMessage> _messages = [
    ChatMessage(
      content: '你好，在吗？',
      isMe: false,
      timestamp: DateTime.now().subtract(const Duration(minutes: 5)),
    ),
    ChatMessage(
      content: '在的，什么事？',
      isMe: true,
      timestamp: DateTime.now().subtract(const Duration(minutes: 4)),
    ),
  ];

  void _sendMessage() {
    final content = _messageController.text.trim();
    if (content.isEmpty) return;

    setState(() {
      _messages.add(
        ChatMessage(
          content: content,
          isMe: true,
          timestamp: DateTime.now(),
        ),
      );
    });

    _messageController.clear();
    _scrollController.animateTo(
      _scrollController.position.maxScrollExtent,
      duration: const Duration(milliseconds: 300),
      curve: Curves.easeOut,
    );
  }

  @override
  Widget build(BuildContext context) {
    final textTheme = Theme.of(context).textTheme;
    final screenWidth = MediaQuery.of(context).size.width;
    final isSmallScreen = screenWidth < 600;

    final double messagePadding = isSmallScreen ? 8 : 16;
    final double messageSpacing = isSmallScreen ? 4 : 8;
    final double bubbleRadius = isSmallScreen ? 12 : 16;

    return Scaffold(
      appBar: AppBar(
        leading: MediaQuery.of(context).size.width <= 800
            ? IconButton(
                icon: const Icon(Icons.arrow_back),
                onPressed: () => Navigator.pop(context),
              )
            : null,
        title: const Text('张三'),
        actions: [
          IconButton(
            icon: const Icon(Icons.more_horiz),
            onPressed: () {
              // TODO: 实现更多功能菜单
            },
          ),
        ],
      ),
      body: Column(
        children: [
          Expanded(
            child: ListView.builder(
              controller: _scrollController,
              padding: EdgeInsets.symmetric(
                horizontal: messagePadding,
                vertical: messageSpacing,
              ),
              itemCount: _messages.length,
              itemBuilder: (context, index) {
                final message = _messages[index];
                return _MessageBubble(
                  message: message,
                  textTheme: textTheme,
                  bubbleRadius: bubbleRadius,
                  messagePadding: messagePadding,
                  messageSpacing: messageSpacing,
                );
              },
            ),
          ),
          Container(
            padding: EdgeInsets.all(messagePadding),
            decoration: BoxDecoration(
              color: Theme.of(context).colorScheme.surface,
              boxShadow: [
                BoxShadow(
                  color: Colors.black.withOpacity(0.1),
                  blurRadius: 4,
                  offset: const Offset(0, -1),
                ),
              ],
            ),
            child: SafeArea(
              child: Row(
                children: [
                  IconButton(
                    icon: const Icon(Icons.add_circle_outline),
                    onPressed: () {
                      // TODO: 实现添加附件功能
                    },
                  ),
                  Expanded(
                    child: TextField(
                      controller: _messageController,
                      style: textTheme.bodyMedium,
                      decoration: InputDecoration(
                        hintText: '输入消息...',
                        hintStyle: textTheme.bodyMedium?.copyWith(
                          color: Colors.grey,
                        ),
                        border: OutlineInputBorder(
                          borderRadius: BorderRadius.circular(bubbleRadius),
                          borderSide: BorderSide.none,
                        ),
                        filled: true,
                        fillColor: Theme.of(context)
                            .colorScheme
                            .surfaceContainerHighest,
                        contentPadding: EdgeInsets.symmetric(
                          horizontal: messagePadding,
                          vertical: messageSpacing,
                        ),
                      ),
                      maxLines: null,
                      textInputAction: TextInputAction.send,
                      onSubmitted: (_) => _sendMessage(),
                    ),
                  ),
                  const SizedBox(width: 8),
                  IconButton(
                    icon: const Icon(Icons.send),
                    onPressed: _sendMessage,
                  ),
                ],
              ),
            ),
          ),
        ],
      ),
    );
  }
}

class ChatMessage {
  final String content;
  final bool isMe;
  final DateTime timestamp;

  ChatMessage({
    required this.content,
    required this.isMe,
    required this.timestamp,
  });
}

class _MessageBubble extends StatelessWidget {
  final ChatMessage message;
  final TextTheme textTheme;
  final double bubbleRadius;
  final double messagePadding;
  final double messageSpacing;

  const _MessageBubble({
    required this.message,
    required this.textTheme,
    required this.bubbleRadius,
    required this.messagePadding,
    required this.messageSpacing,
  });

  @override
  Widget build(BuildContext context) {
    return Align(
      alignment: message.isMe ? Alignment.centerRight : Alignment.centerLeft,
      child: Container(
        margin: EdgeInsets.only(
          left: message.isMe ? 64 : 0,
          right: message.isMe ? 0 : 64,
          bottom: messageSpacing,
        ),
        padding: EdgeInsets.symmetric(
          horizontal: messagePadding,
          vertical: messageSpacing,
        ),
        decoration: BoxDecoration(
          color: message.isMe
              ? Theme.of(context).colorScheme.primary
              : Theme.of(context).colorScheme.surfaceContainerHighest,
          borderRadius: BorderRadius.circular(bubbleRadius),
        ),
        child: Text(
          message.content,
          style: textTheme.bodyMedium?.copyWith(
            color: message.isMe ? Colors.white : null,
          ),
        ),
      ),
    );
  }
}
