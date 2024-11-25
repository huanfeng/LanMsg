import 'package:flutter/material.dart';

class CustomNavigationBar extends StatelessWidget {
  final int selectedIndex;
  final Function(int) onItemSelected;
  final bool isVertical;

  const CustomNavigationBar({
    super.key,
    required this.selectedIndex,
    required this.onItemSelected,
    required this.isVertical,
  });

  @override
  Widget build(BuildContext context) {
    final items = [
      const NavigationDestination(
        icon: Icon(Icons.message_outlined),
        selectedIcon: Icon(Icons.message),
        label: '消息',
      ),
      const NavigationDestination(
        icon: Icon(Icons.contacts_outlined),
        selectedIcon: Icon(Icons.contacts),
        label: '联系人',
      ),
      const NavigationDestination(
        icon: Icon(Icons.explore_outlined),
        selectedIcon: Icon(Icons.explore),
        label: '发现',
      ),
      const NavigationDestination(
        icon: Icon(Icons.person_outline),
        selectedIcon: Icon(Icons.person),
        label: '我的',
      ),
    ];

    if (isVertical) {
      return NavigationRail(
        selectedIndex: selectedIndex,
        onDestinationSelected: onItemSelected,
        labelType: NavigationRailLabelType.all,
        destinations: items
            .map((item) => NavigationRailDestination(
                  icon: item.icon,
                  selectedIcon: item.selectedIcon,
                  label: Text(item.label),
                ))
            .toList(),
      );
    }

    return NavigationBar(
      selectedIndex: selectedIndex,
      onDestinationSelected: onItemSelected,
      destinations: items,
    );
  }
}
