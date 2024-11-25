import 'package:flutter/material.dart';
import 'package:flutter_screenutil/flutter_screenutil.dart';
import '../widgets/chat_list.dart';
import '../widgets/chat_detail.dart';
import '../widgets/navigation_bar.dart';

class HomeScreen extends StatefulWidget {
  const HomeScreen({super.key});

  @override
  State<HomeScreen> createState() => _HomeScreenState();
}

class _HomeScreenState extends State<HomeScreen> {
  int _selectedIndex = 0;
  String? _selectedChatId;

  void _onNavigationItemSelected(int index) {
    setState(() {
      _selectedIndex = index;
    });
  }

  void _onChatSelected(String chatId) {
    setState(() {
      _selectedChatId = chatId;
    });
  }

  @override
  Widget build(BuildContext context) {
    final width = MediaQuery.of(context).size.width;
    final isLargeScreen = width > 800;

    if (isLargeScreen) {
      return Scaffold(
        body: Row(
          children: [
            CustomNavigationBar(
              selectedIndex: _selectedIndex,
              onItemSelected: _onNavigationItemSelected,
              isVertical: true,
            ),
            const VerticalDivider(width: 1),
            Expanded(
              flex: 3,
              child: ChatList(
                onChatSelected: _onChatSelected,
                selectedChatId: _selectedChatId,
              ),
            ),
            const VerticalDivider(width: 1),
            if (_selectedChatId != null)
              Expanded(
                flex: 5,
                child: ChatDetail(chatId: _selectedChatId!),
              )
            else
              Expanded(
                flex: 5,
                child: Center(
                  child: Text(
                    '选择一个聊天',
                    style: TextStyle(
                      fontSize: 16.sp,
                      color: Colors.grey,
                    ),
                  ),
                ),
              ),
          ],
        ),
      );
    }

    return Scaffold(
      body: ChatList(
        onChatSelected: _onChatSelected,
        selectedChatId: _selectedChatId,
      ),
      bottomNavigationBar: CustomNavigationBar(
        selectedIndex: _selectedIndex,
        onItemSelected: _onNavigationItemSelected,
        isVertical: false,
      ),
    );
  }
}
