package dev.herdr.demo;

import android.app.Activity;
import android.net.Uri;
import android.os.Bundle;
import android.util.Log;
import android.view.Gravity;
import android.view.View;
import android.widget.Button;
import android.widget.EditText;
import android.widget.LinearLayout;
import android.widget.TextView;

/** A sign-in screen with a bug, for trying herdr-android: Sign in crashes, Freeze causes an ANR. */
public class MainActivity extends Activity {
    private static final String TAG = "HerdrDemo";

    @Override
    protected void onCreate(Bundle state) {
        super.onCreate(state);
        LinearLayout root = new LinearLayout(this);
        root.setOrientation(LinearLayout.VERTICAL);
        root.setGravity(Gravity.CENTER_HORIZONTAL);
        // Android 15+ draws apps edge to edge: keep content clear of the status and nav bars.
        root.setOnApplyWindowInsetsListener((v, insets) -> {
            v.setPadding(48, insets.getSystemWindowInsetTop() + 48, 48, insets.getSystemWindowInsetBottom() + 48);
            return insets;
        });

        TextView title = new TextView(this);
        title.setText("Herdr Demo");
        title.setTextSize(28);
        root.addView(title);

        TextView status = new TextView(this);
        Uri link = getIntent().getData();
        status.setText(link == null ? "Signed out" : "Opened from " + link);
        status.setId(android.R.id.text1);
        root.addView(status);

        EditText email = new EditText(this);
        email.setHint("Email");
        email.setId(android.R.id.edit);
        root.addView(email);

        Button signIn = new Button(this);
        signIn.setText("Sign in");
        signIn.setId(android.R.id.button1);
        signIn.setOnClickListener(v -> {
            Log.i(TAG, "signing in as " + email.getText());
            // The bug: an empty email isn't handled.
            if (email.getText().length() == 0) {
                throw new IllegalStateException("sign in failed: no email");
            }
            status.setText("Signed in as " + email.getText());
        });
        root.addView(signIn);

        Button freeze = new Button(this);
        freeze.setText("Freeze");
        freeze.setId(android.R.id.button2);
        freeze.setOnClickListener(v -> {
            Log.w(TAG, "blocking the main thread");
            try {
                Thread.sleep(12_000);
            } catch (InterruptedException ignored) {
            }
        });
        root.addView(freeze);

        setContentView(root);
        // Dark status bar icons on the light background.
        getWindow().getDecorView().setSystemUiVisibility(View.SYSTEM_UI_FLAG_LIGHT_STATUS_BAR);
        Log.i(TAG, "started");
    }
}
