import 'package:flutter/material.dart';
import 'package:lanmsg/models/message.dart';
import 'package:lanmsg/services/message_service.dart';
import 'package:lanmsg/widgets/message_list.dart';
import 'package:provider/provider.dart';

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
  late final MessageService _messageService;
  List<Message> _messages = [];

  @override
  void initState() {
    super.initState();
    _messageService = context.read<MessageService>();
    _loadMessages();
  }

  Future<void> _loadMessages() async {
    final messages = await _messageService.getMessagesForChat(widget.chatId);
    setState(() {
      _messages = messages;
    });
    _scrollToBottom();
  }

  void _scrollToBottom() {
    if (_scrollController.hasClients) {
      _scrollController.animateTo(
        _scrollController.position.maxScrollExtent,
        duration: const Duration(milliseconds: 300),
        curve: Curves.easeOut,
      );
    }
  }

  Future<void> _sendMessage() async {
    final content = _messageController.text.trim();
    if (content.isEmpty) return;

    final message = Message(
      id: 'msg_${DateTime.now().millisecondsSinceEpoch}',
      chatId: widget.chatId,
      senderId: 'user1', // 当前用户ID
      content: content,
      timestamp: DateTime.now(),
      isRead: true,
    );

    await _messageService.saveMessage(message);
    _messageController.clear();
    await _loadMessages();
  }

  @override
  Widget build(BuildContext context) {
    final textTheme = Theme.of(context).textTheme;
    final screenWidth = MediaQuery.of(context).size.width;
    final isSmallScreen = screenWidth < 600;

    final double messagePadding = isSmallScreen ? 8 : 16;
    final double messageSpacing = isSmallScreen ? 4 : 8;

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
            child: MessageList(
              messages: _messages,
              currentUserId: 'user1', // 当前用户ID
            ),
          ),
          Container(
            padding: const EdgeInsets.all(8),
            decoration: BoxDecoration(
              color: Theme.of(context).cardColor,
              boxShadow: [
                BoxShadow(
                  color: Colors.black.withOpacity(0.1),
                  blurRadius: 4,
                  offset: const Offset(0, -2),
                ),
              ],
            ),
            child: Row(
              children: [
                IconButton(
                  icon: const Icon(Icons.add),
                  onPressed: () {
                    // TODO: 实现添加附件功能
                  },
                ),
                Expanded(
                  child: TextField(
                    controller: _messageController,
                    decoration: const InputDecoration(
                      hintText: '输入消息...',
                      border: InputBorder.none,
                    ),
                    onSubmitted: (_) => _sendMessage(),
                  ),
                ),
                IconButton(
                  icon: const Icon(Icons.send),
                  onPressed: _sendMessage,
                ),
              ],
            ),
          ),
        ],
      ),
    );
  }
}
