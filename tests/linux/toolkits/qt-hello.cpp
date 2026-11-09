/*
 * qt-hello: a small Qt Widgets app for Poly's desktop (ADR 0120), run
 * unchanged against the Qt 6 the root disk copies from the build image,
 * on Qt's Wayland platform plugin (QT_QPA_PLATFORM=wayland). Written from
 * Qt's public documentation (doc.qt.io); no moc needed (no Q_OBJECT).
 *
 * One 360x240 window titled "Poly Qt" holding a push button coloured by a
 * style sheet (QT_A, then QT_B once clicked). It reports the platform,
 * the window shown and first painted, clicks and keys on standard output
 * for the boot test (tools/xtask/src/toolkits.rs); Q or the window's
 * close ends it.
 *
 * Not in tests/linux/ itself: the diff runner compares those with real
 * Linux.
 */
#include <QApplication>
#include <QKeyEvent>
#include <QPaintEvent>
#include <QPushButton>
#include <QShowEvent>
#include <QVBoxLayout>
#include <QWidget>
#include <cstdio>

/* The button's two colours (toolkits.rs checks them on the screen). */
static const char *QT_A = "#7a5a1a";
static const char *QT_B = "#1a6a7a";

static QString sheet(const char *colour) {
    return QString("QPushButton { background-color: %1; color: white; border: none; }").arg(colour);
}

class Window : public QWidget {
public:
    Window() {
        setWindowTitle("Poly Qt");
        resize(360, 240);
        auto *layout = new QVBoxLayout(this);
        layout->setContentsMargins(0, 0, 0, 0);
        button = new QPushButton("Click me", this);
        button->setSizePolicy(QSizePolicy::Expanding, QSizePolicy::Expanding);
        button->setStyleSheet(sheet(QT_A));
        button->setFocusPolicy(Qt::NoFocus);
        layout->addWidget(button);
        QObject::connect(button, &QPushButton::clicked, [this]() {
            clicked = !clicked;
            button->setStyleSheet(sheet(clicked ? QT_B : QT_A));
            button->setText(clicked ? "Clicked" : "Click me");
            std::printf("qt-hello: clicked\n");
        });
    }

protected:
    void showEvent(QShowEvent *) override { std::printf("qt-hello: shown\n"); }

    void paintEvent(QPaintEvent *) override {
        if (!painted) {
            painted = true;
            std::printf("qt-hello: drawn\n");
        }
    }

    void keyPressEvent(QKeyEvent *e) override {
        QString text = e->text();
        std::printf("qt-hello: key %d %s\n", e->key(), text.toUtf8().constData());
        if (e->key() == Qt::Key_Q) {
            close();
        }
    }

private:
    QPushButton *button;
    bool clicked = false;
    bool painted = false;
};

int main(int argc, char **argv) {
    std::setvbuf(stdout, nullptr, _IOLBF, 0);
    std::printf("qt-hello: starting\n");
    QApplication app(argc, argv);
    std::printf("qt-hello: Qt %s, platform %s\n", qVersion(), QGuiApplication::platformName().toUtf8().constData());
    Window w;
    w.show();
    int status = app.exec();
    std::printf("qt-hello: closed, status %d\n", status);
    return status;
}
