QT += core gui widgets

greaterThan(QT_MAJOR_VERSION, 4): QT += widgets

CONFIG += c++11

TARGET = wftpg-gui
TEMPLATE = app

SOURCES += \
    main.cpp \
    mainwindow.cpp \
    wftpg_bridge.cpp

HEADERS += \
    mainwindow.h \
    wftpg_bridge.h

LIBS += -L../target/release -lwftpg
INCLUDEPATH += ..

target.path = /usr/bin
INSTALLS += target
